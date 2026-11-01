//! The older records of a member when it moves to another archive (D-156).
//!
//! Inside an archive a shadow follows its winner as raw bytes. Between archives a move is a copy and a removal, and the
//! other archive may be of another format, so what travels is what a member is: content, time, attributes. Each record
//! is taken out of the source on its own, decoded into the cache, and joins the moved member's overlay item, and the
//! rebuild of the target writes them right before it, oldest first, so that the member still wins its path.
//!
//! Neither format's reader names these records. A zip's are copied raw to a small zip of their own under numbered
//! names and indexed there, which also reaches the records the crate hides; a tar's blocks are cut out of the stream
//! and read back as a tar. What cannot be read back as a member (an encrypted zip record, a tar hard link, whose
//! content is another record's) stays behind, and the caller says how many.

use std::collections::HashSet;
use std::io::{self, Read};
use std::sync::Arc;

use hikasa_core::{Ctl, FileTime, Spec};
use tar::EntryType;

use super::cache::{CacheFile, CacheWriter};
use super::codec::{self, Codec, Pass, corrupt};
use super::index::{DataRef, MemberKind, Node};
use super::overlay::{Item, Origin};
use super::zip_read::{directory_limit, index_zip};
use super::zip_write::{Patch, ZipOut, read_central, record_starts};
use super::{ArchiveConfig, ArchiveFormat, Core, Index};
use crate::entry::{Attrs, UnixMeta};
use crate::error::{VfsError, VfsOp};
use crate::vfs::ReadSeek;

/// What came out of a source archive.
pub(super) struct Older {
    /// Oldest first.
    pub(super) items: Vec<Item>,
    /// Records that cannot leave.
    pub(super) left: u64,
}

impl Core {
    /// The older records of `node`, a member of `archive`.
    pub(super) fn older_of(
        &self,
        archive: &Spec,
        index: &Index,
        node: &Node,
        config: &ArchiveConfig,
        ctl: Ctl<'_>,
    ) -> Result<Older, VfsError> {
        let text = archive.canonical();
        let pass = Pass {
            spec: &text,
            limits: config.limits,
            ctl,
            zone: (config.clock)().time_zone().clone(),
        };
        if index.format == ArchiveFormat::Zip {
            self.older_of_zip(archive, index, node, config, &pass)
        } else {
            self.older_of_tar(archive, index, node, config, &pass)
        }
    }

    /// A stage for one record.
    fn older_stage(&self, config: &ArchiveConfig, text: &str) -> Result<CacheWriter, VfsError> {
        self.cache
            .stage(&config.cache_root, config.limits.cache_max, text)
    }

    fn older_of_zip(
        &self,
        archive: &Spec,
        index: &Index,
        node: &Node,
        config: &ArchiveConfig,
        pass: &Pass<'_>,
    ) -> Result<Older, VfsError> {
        let text = pass.spec;
        let Some(data) = &index.zip else {
            return Err(corrupt(text, "inconsistent index"));
        };
        let tree = &index.tree;
        let numbered: HashSet<u32> = tree
            .shadows
            .iter()
            .filter(|shadow| shadow.winner == node.record)
            .map(|shadow| shadow.record)
            .collect();
        let mut wanted: HashSet<u64> = tree
            .hidden
            .iter()
            .filter(|(_, winner)| *winner == node.record)
            .map(|(at, _)| *at)
            .collect();
        wanted.extend(data.directory_offsets().filter_map(|(at, record)| {
            let record = u32::try_from(record).ok()?;
            numbered.contains(&record).then_some(at)
        }));

        // Each record, under its number, in a zip of their own.
        let mut input = self.open_container(archive, config, pass.ctl)?;
        let start = data.directory_start();
        let headers = read_central(&mut *input, start, directory_limit(&pass.limits), text)?;
        let starts = record_starts(&headers);
        let mut stage = self.older_stage(config, text)?;
        let mut count = 0usize;
        {
            let mut out = ZipOut::new(&mut stage, pass.ctl, text, data.container_len());
            for header in headers.iter().filter(|h| wanted.contains(&h.at)) {
                let patch = Patch {
                    name: Some(count.to_string().into_bytes()),
                    ..Patch::default()
                };
                out.record(&mut *input, header, &patch, &starts, start)?;
                count += 1;
            }
            out.finish(&[], None)?;
        }
        drop(input);
        let records = stage.finish_staged()?;

        let open = |file: &Arc<CacheFile>| -> Result<Box<dyn ReadSeek>, VfsError> {
            Ok(Box::new(file.reader()?))
        };
        let (records_tree, zip) = index_zip(open(&records)?, records.len(), pass)?;
        let _lease = zip.lease(|| open(&records))?;
        let mut nodes: Vec<&Node> = records_tree.nodes.iter().collect();
        nodes.sort_by_key(|node| node.path().parse::<usize>().unwrap_or(usize::MAX));
        let total = std::sync::atomic::AtomicU64::new(0);
        let mut older = Older {
            items: Vec::with_capacity(count),
            left: 0,
        };
        for record in nodes {
            match (record.kind, record.data) {
                (MemberKind::Symlink, _) if !record.encrypted => older.items.push(Item::older(
                    MemberKind::Symlink,
                    Origin::Fresh,
                    record.size,
                    record.mtime,
                    record.attrs,
                    record.link_target.clone(),
                )),
                (MemberKind::File, DataRef::Zip { index: at }) if !record.encrypted => {
                    let mut content = self.older_stage(config, text)?;
                    zip.decode(at, record.size, pass, &total, &mut |bytes| {
                        content.write(bytes)
                    })?;
                    older.items.push(Item::older(
                        MemberKind::File,
                        Origin::Staged(content.finish_staged()?),
                        record.size,
                        record.mtime,
                        record.attrs,
                        None,
                    ));
                }
                _ => older.left += 1,
            }
        }
        Ok(older)
    }

    fn older_of_tar(
        &self,
        archive: &Spec,
        index: &Index,
        node: &Node,
        config: &ArchiveConfig,
        pass: &Pass<'_>,
    ) -> Result<Older, VfsError> {
        let text = pass.spec;
        let codec = Codec::of(index.format);
        let mut limits = pass.limits;
        if codec.is_none() {
            limits.stream_max = u64::MAX;
        }
        let pass = Pass {
            spec: text,
            limits,
            ctl: pass.ctl,
            zone: pass.zone.clone(),
        };
        let mut records: Vec<u32> = index
            .tree
            .shadows
            .iter()
            .filter(|shadow| shadow.winner == node.record)
            .map(|shadow| shadow.record)
            .collect();
        records.sort_unstable();
        let mut older = Older {
            items: Vec::with_capacity(records.len()),
            left: 0,
        };
        for record in records {
            let Some(DataRef::Tar {
                first_block, end, ..
            }) = index
                .tree
                .records
                .get(record as usize)
                .map(|record| record.data)
            else {
                return Err(corrupt(text, "inconsistent index"));
            };
            let mut blocks = self.older_stage(config, text)?;
            let len = end.saturating_sub(first_block);
            self.decode_range(
                codec,
                archive,
                first_block,
                len,
                config,
                &pass,
                &mut |bytes| blocks.write(bytes),
            )?;
            let blocks = blocks.finish_staged()?;
            match self.older_of_blocks(&blocks, config, text)? {
                Some(item) => older.items.push(item),
                None => older.left += 1,
            }
        }
        Ok(older)
    }

    /// The member in `blocks`, one record of a tar with what heads it; `None` when it has no content of its own.
    fn older_of_blocks(
        &self,
        blocks: &Arc<CacheFile>,
        config: &ArchiveConfig,
        text: &str,
    ) -> Result<Option<Item>, VfsError> {
        let fail = |error: io::Error| VfsError::from_io(text, VfsOp::Read, error);
        let mut archive = tar::Archive::new(blocks.reader()?);
        let mut entries = archive.entries().map_err(fail)?;
        let Some(entry) = entries.next() else {
            return Err(corrupt(text, "a record without a header"));
        };
        let mut entry = entry.map_err(fail)?;
        let header = entry.header();
        let kind = header.entry_type();
        let mtime = header
            .mtime()
            .ok()
            .and_then(|seconds| i64::try_from(seconds).ok())
            .and_then(|seconds| FileTime::from_unix(seconds, 0));
        let attrs = header.mode().map_or(Attrs::None, |mode| Attrs::Unix {
            meta: UnixMeta {
                mode,
                uid: header
                    .uid()
                    .ok()
                    .and_then(|id| u32::try_from(id).ok())
                    .unwrap_or(0),
                gid: header
                    .gid()
                    .ok()
                    .and_then(|id| u32::try_from(id).ok())
                    .unwrap_or(0),
            },
        });
        match kind {
            EntryType::Regular | EntryType::Continuous | EntryType::GNUSparse => {
                let mut content = self.older_stage(config, text)?;
                let mut buffer = vec![0u8; codec::BUFFER];
                let mut size = 0u64;
                loop {
                    let n = match entry.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(fail(e)),
                    };
                    content.write(buffer.get(..n).unwrap_or_default())?;
                    size += n as u64;
                }
                Ok(Some(Item::older(
                    MemberKind::File,
                    Origin::Staged(content.finish_staged()?),
                    size,
                    mtime,
                    attrs,
                    None,
                )))
            }
            EntryType::Symlink => {
                let target = entry.link_name().map_err(fail)?;
                let target = target.map(|t| t.to_string_lossy().into_owned().into_boxed_str());
                Ok(Some(Item::older(
                    MemberKind::Symlink,
                    Origin::Fresh,
                    0,
                    mtime,
                    attrs,
                    target,
                )))
            }
            _ => Ok(None),
        }
    }
}
