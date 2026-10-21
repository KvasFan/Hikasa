//! JSON goldens of the session-owned DTOs (§4.1, §4.2, §4.3.1, §4.3.4): the envelope, `hikasa.quick.v1`,
//! `format list/show/detect/tokens`, `vfs ls/stat/cat`, the gui DTOs and `hikasa.version.v1`. Temporary paths
//! become `<TMP>` and the version `<VERSION>`; machine-dependent Unix ids become `<UID>`/`<GID>`.

mod common;

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;

use common::{job, os, write};
use hikasa_core::{ExitCode, JobKind, Warning, WarningCode};
use hikasa_session::api::dto::{
    FORMAT_LIST_SCHEMA, GuiCloseDto, GuiListDto, GuiQuitDto, JsonEnvelope, QUICK_SCHEMA, QuickDto,
    SCHEMAS, VersionDto, write_json,
};
use hikasa_session::api::{ApiOp, FormatTokensRequest, QuickMethod, QuickRequest};
use hikasa_session::ipc::{ViewId, ViewInfo, ViewKind};
use hikasa_session::{ApiError, HIKASA_VERSION};
use serde::Serialize;
use serde_json::Value;

/// Pretty JSON of `value` (declaration order kept) with `root` replaced by `<TMP>` (the separators after it
/// normalised to `/`), the version by `<VERSION>`, and machine-dependent Unix ids and birth times blanked.
fn normalized<T: Serialize>(value: &T, root: &Path) -> Vec<u8> {
    let mut out = Vec::new();
    write_json(&mut out, value).unwrap();
    let text = String::from_utf8(out).unwrap();
    let escaped_root = serde_json::to_string(&root.to_string_lossy()).unwrap();
    let escaped_root = &escaped_root[1..escaped_root.len() - 1];
    let version = format!("\"{HIKASA_VERSION}\"");
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut line = raw.replace(&version, "\"<VERSION>\"");
        while let Some(at) = line.find(escaped_root) {
            let head = line[..at].to_owned();
            let tail = line[at + escaped_root.len()..].to_owned();
            let end = tail.find(['"', ':', ' ']).unwrap_or(tail.len());
            let (path, rest) = tail.split_at(end);
            line = format!("{head}<TMP>{}{rest}", path.replace("\\\\", "/"));
        }
        let indent_len = line.len() - line.trim_start().len();
        let (indent, trimmed) = line.split_at(indent_len);
        let mut blanked = None;
        for (key, blank) in [
            ("\"uid\": ", "\"<UID>\""),
            ("\"gid\": ", "\"<GID>\""),
            ("\"ctime\": \"", "\"<CTIME>\""),
        ] {
            if trimmed.starts_with(key) && !trimmed.contains("null") {
                let comma = if trimmed.ends_with(',') { "," } else { "" };
                let name = &key[..key.find(':').unwrap() + 1];
                blanked = Some(format!("{indent}{name} {blank}{comma}"));
            }
        }
        lines.push(blanked.unwrap_or(line));
    }
    (lines.join("\n") + "\n").into_bytes()
}

fn golden(name: &str, actual: &[u8]) {
    common::assert_golden(&common::fixtures("dto").join(name), actual);
}

#[test]
fn quick_envelopes() {
    let tmp = tempfile::tempdir().unwrap();
    let service = common::service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "x.c", b"int x; /* a */\n");
    let b = write(&w, "y.c", b"int x; /* b */\n");
    let req = QuickRequest {
        left: os(&a),
        right: os(&b),
        method: QuickMethod::Rules,
        ..QuickRequest::default()
    };
    let resp = service
        .quick(&req, &job(&service, JobKind::QuickCompare))
        .unwrap();
    let envelope = JsonEnvelope::success(
        QUICK_SCHEMA,
        resp.exit_code,
        QuickDto::new(&resp, req.ignore_unimportant),
        resp.outcome.warnings.clone(),
    );
    golden("quick_ok.json", &normalized(&envelope, tmp.path()));
    let crc = service
        .quick(
            &QuickRequest {
                method: QuickMethod::Crc,
                ..req.clone()
            },
            &job(&service, JobKind::QuickCompare),
        )
        .unwrap();
    let dto = serde_json::to_value(QuickDto::new(&crc, false)).unwrap();
    assert_eq!(dto["format"], Value::Null, "null for size, crc and binary");
    assert_eq!(dto["method"], "crc");
    let err = service
        .quick(
            &QuickRequest {
                right: os(&w.join("missing.c")),
                ..req
            },
            &job(&service, JobKind::QuickCompare),
        )
        .unwrap_err();
    let failure: JsonEnvelope<QuickDto> = JsonEnvelope::failure(
        QUICK_SCHEMA,
        &err,
        ApiOp::Quick,
        vec![Warning {
            code: WarningCode::FormatMismatch,
            message: "m".into(),
            spec: None,
        }],
    );
    golden("quick_error.json", &normalized(&failure, tmp.path()));
    let text = String::from_utf8(normalized(&failure, tmp.path())).unwrap();
    assert!(text.ends_with("}\n"), "one document followed by a newline");
}

#[test]
fn format_documents() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    write(
        &config,
        "formats/upper.toml",
        common::EXTERNAL_FORMAT.as_bytes(),
    );
    write(
        &config,
        "formats/badre.toml",
        b"schema_version = 1\nid = \"badre\"\nname = \"Bad regex\"\nkind = \"text\"\nmask = \"*.bad\"\n\
          [[grammar]]\nelement = \"X\"\nkind = \"basic\"\ntext = \"(\"\nregex = true\n",
    );
    let stdin = common::blob(b"", Some("hint.py"));
    let service = common::service_with(
        tmp.path(),
        Some(stdin),
        Arc::new(common::UpperRunner::default()),
    );
    let list = service.format_list();
    let envelope = JsonEnvelope::success(
        FORMAT_LIST_SCHEMA,
        ExitCode::Success,
        list,
        service.startup_warnings().to_vec(),
    );
    golden("format_list.json", &normalized(&envelope, tmp.path()));
    for id in ["ini-reg", "sorted", "upper", "badre", "csv"] {
        let show = service.format_show(id).unwrap();
        golden(
            &format!("format_show_{id}.json"),
            &normalized(&show, tmp.path()),
        );
    }
    assert!(
        service
            .format_show("badre")
            .unwrap()
            .compile_error
            .is_some()
    );
    assert!(
        service
            .format_show("ini-reg")
            .unwrap()
            .compile_error
            .is_none()
    );
    assert_eq!(
        service.format_show("C, C++, C#, ObjC Source").unwrap().id,
        "c-cpp",
        "by case-insensitive name"
    );
    assert!(matches!(
        service.format_show("nope"),
        Err(ApiError::Format(_))
    ));
    assert!(
        service
            .format_toml("ini-reg")
            .unwrap()
            .contains("id = \"ini-reg\"")
    );
    let mut detects = Vec::new();
    for file in ["dir/x.c", "-", "README", "data.CSV"] {
        detects.push(
            service
                .format_detect(OsString::from(file).as_os_str())
                .unwrap(),
        );
    }
    golden("format_detect.json", &normalized(&detects, tmp.path()));
    let w = tmp.path().join("work");
    let file = write(
        &w,
        "t.c",
        b"int a; /* one\n two */ int b;\nchar *s = \"x\";\n",
    );
    let tokens = service
        .format_tokens(
            &FormatTokensRequest {
                file: os(&file),
                lines: Some(2..=9),
                ..FormatTokensRequest::default()
            },
            &job(&service, JobKind::FormatTokens),
        )
        .unwrap();
    golden("format_tokens.json", &normalized(&tokens, tmp.path()));
    let forced = service
        .format_tokens(
            &FormatTokensRequest {
                file: os(&file),
                format: Some("python".into()),
                encoding: Some("windows-1252".into()),
                lines: Some(1..=1),
            },
            &job(&service, JobKind::FormatTokens),
        )
        .unwrap();
    let v = serde_json::to_value(&forced).unwrap();
    assert_eq!(v["format"]["reason"]["reason"], "forced");
    assert_eq!(v["encoding"]["source"], "override");
    assert_eq!(v["lines"].as_array().unwrap().len(), 1);
}

/// Linux only: the capability set and the stat facts are platform defaults (§1.2.3).
#[cfg(target_os = "linux")]
#[test]
fn vfs_documents() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let service = common::service(tmp.path());
    let dir = tmp.path().join("work").join("v");
    let file = write(&dir, "b.txt", b"hello\n");
    write(&dir, "a.bin", &[0, 1, 2, 255]);
    std::fs::create_dir(dir.join("sub")).unwrap();
    let links = tmp.path().join("work").join("links");
    std::fs::create_dir_all(&links).unwrap();
    std::os::unix::fs::symlink(&file, links.join("link")).unwrap();
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_792_584_000);
    for name in ["b.txt", "a.bin", "sub"] {
        let path = dir.join(name);
        let mode = if name == "sub" { 0o750 } else { 0o640 };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        std::fs::File::open(&path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }
    let control = job(&service, JobKind::VfsList);
    let list = service
        .vfs_ls(dir.as_os_str(), hikasa_vfs::FollowLinks::No, &control)
        .unwrap();
    golden("vfs_list.json", &normalized(&list, tmp.path()));
    let stat = service
        .vfs_stat(file.as_os_str(), hikasa_vfs::FollowLinks::No)
        .unwrap();
    golden("vfs_stat.json", &normalized(&stat, tmp.path()));
    let followed = service
        .vfs_stat(links.join("link").as_os_str(), hikasa_vfs::FollowLinks::Yes)
        .unwrap();
    assert!(followed.entry.followed);
    let mut out = Vec::new();
    let n = service
        .vfs_cat(
            dir.join("a.bin").as_os_str(),
            &mut out,
            &job(&service, JobKind::VfsRead),
        )
        .unwrap();
    assert_eq!(n, 4);
    let cat = hikasa_session::api::dto::VfsCatDto::new(
        dir.join("a.bin").to_string_lossy().into_owned(),
        &out,
    );
    golden("vfs_cat.json", &normalized(&cat, tmp.path()));
    assert!(matches!(
        service.vfs_ls(
            dir.join("nope").as_os_str(),
            hikasa_vfs::FollowLinks::No,
            &control
        ),
        Err(ApiError::Vfs(_))
    ));
}

#[test]
fn gui_and_version_documents() {
    let tmp = tempfile::tempdir().unwrap();
    let list = GuiListDto {
        views: vec![ViewInfo {
            view: ViewId(3),
            kind: ViewKind::FolderCompare,
            title: "a | b".into(),
            specs: vec!["/a".into(), "/b".into()],
        }],
    };
    golden("gui_list.json", &normalized(&list, tmp.path()));
    golden(
        "gui_close.json",
        &normalized(&GuiCloseDto { view: 3 }, tmp.path()),
    );
    golden(
        "gui_quit.json",
        &normalized(&GuiQuitDto::default(), tmp.path()),
    );
    let version = VersionDto::current();
    assert_eq!(version.schemas, SCHEMAS.to_vec());
    golden("version.json", &normalized(&version, tmp.path()));
}
