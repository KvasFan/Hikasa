//! IPC framing and protocol v1 (§4.4, D-46; §6.2.8 "IPC"): framing properties, serde round trips of every
//! message, the wire examples of §4.4.3 as goldens, forward compatibility and the two-phase decode.

mod common;

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use hikasa_core::{CompareVerdict, ContentKind};
use hikasa_folder::FolderRulesPatch;
use hikasa_session::ipc::{
    CloseReason, Envelope, IpcError, IpcErrorCode, MAX_FRAME_LEN, OpenOptions, ReadOnly, Request,
    Response, ServerInfo, ViewId, ViewInfo, ViewKind, ViewResult, read_frame, recv_request,
    recv_response, send, write_frame,
};
use hikasa_text::{AlgorithmChoice, TextRulesPatch, UnimportantItem};
use proptest::prelude::*;

/// A reader that returns one byte per `read` call.
struct OneByte<R>(R);

impl<R: Read> Read for OneByte<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match buf.first_mut() {
            Some(slot) => {
                let mut one = [0u8; 1];
                let n = self.0.read(&mut one)?;
                if n == 1 {
                    *slot = one[0];
                }
                Ok(n)
            }
            None => Ok(0),
        }
    }
}

fn framed(payloads: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in payloads {
        write_frame(&mut out, p).unwrap();
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Arbitrary payloads up to 64 KiB round-trip, also through a 1-byte-at-a-time reader.
    #[test]
    fn p_ipc_frames_round_trip(payloads in prop::collection::vec(prop::collection::vec(any::<u8>(), 1..65_536usize), 1..4)) {
        let bytes = framed(&payloads);
        let mut whole = Cursor::new(bytes.clone());
        let mut slow = OneByte(Cursor::new(bytes));
        for p in &payloads {
            let fast = read_frame(&mut whole, MAX_FRAME_LEN).unwrap();
            let byte_by_byte = read_frame(&mut slow, MAX_FRAME_LEN).unwrap();
            prop_assert_eq!(fast.as_ref(), Some(p));
            prop_assert_eq!(byte_by_byte.as_ref(), Some(p));
        }
        prop_assert!(read_frame(&mut whole, MAX_FRAME_LEN).unwrap().is_none());
        prop_assert!(read_frame(&mut slow, MAX_FRAME_LEN).unwrap().is_none());
    }

    /// A frame cut anywhere inside is `ConnectionLost`; cut at a boundary it is a clean EOF.
    #[test]
    fn p_ipc_truncation(payload in prop::collection::vec(any::<u8>(), 1..2048usize), cut in 1usize..2052) {
        let bytes = framed(std::slice::from_ref(&payload));
        let cut = cut.min(bytes.len() - 1);
        let mut r = Cursor::new(bytes[..cut].to_vec());
        prop_assert!(matches!(read_frame(&mut r, MAX_FRAME_LEN), Err(IpcError::ConnectionLost)));
    }
}

#[test]
fn frame_limits() {
    // Oversized announced length.
    let mut bytes = (MAX_FRAME_LEN + 1).to_le_bytes().to_vec();
    bytes.extend_from_slice(b"{}");
    assert!(matches!(
        read_frame(&mut Cursor::new(bytes), MAX_FRAME_LEN),
        Err(IpcError::FrameTooLarge { len, max }) if len == MAX_FRAME_LEN + 1 && max == MAX_FRAME_LEN
    ));
    // A caller-chosen smaller limit.
    let bytes = framed(&[vec![b'x'; 100]]);
    assert!(matches!(
        read_frame(&mut Cursor::new(bytes), 99),
        Err(IpcError::FrameTooLarge { len: 100, max: 99 })
    ));
    // Zero length.
    assert!(matches!(
        read_frame(&mut Cursor::new(0u32.to_le_bytes().to_vec()), MAX_FRAME_LEN),
        Err(IpcError::Protocol(_))
    ));
    // Writers refuse empty and oversized payloads.
    assert!(matches!(
        write_frame(&mut Vec::new(), b""),
        Err(IpcError::Protocol(_))
    ));
    let huge = vec![0u8; MAX_FRAME_LEN as usize + 1];
    assert!(matches!(
        write_frame(&mut Vec::new(), &huge),
        Err(IpcError::FrameTooLarge { .. })
    ));
    assert!(write_frame(&mut Vec::new(), &huge[..MAX_FRAME_LEN as usize]).is_ok());
    // Clean EOF and EOF inside the header.
    assert!(
        read_frame(&mut Cursor::new(Vec::new()), MAX_FRAME_LEN)
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        read_frame(&mut Cursor::new(vec![5, 0]), MAX_FRAME_LEN),
        Err(IpcError::ConnectionLost)
    ));
    // The header is little-endian.
    let bytes = framed(&[b"abc".to_vec()]);
    assert_eq!(&bytes[..4], &[3, 0, 0, 0]);
}

fn sample_options() -> OpenOptions {
    let mut extra = BTreeMap::new();
    extra.insert("automerge".to_owned(), String::new());
    extra.insert("vcs1".to_owned(), "r42".to_owned());
    OpenOptions {
        titles: [Some("L".into()), None, None, Some("O".into())],
        read_only: ReadOnly {
            all: false,
            left: true,
            right: false,
        },
        filters: Some("*.c;-obj\\".into()),
        expand_all: true,
        format: Some("c-cpp".into()),
        ignore_unimportant: true,
        text_rules_file: Some("/r/text.toml".into()),
        text_rules: Some(TextRulesPatch {
            algorithm: Some(AlgorithmChoice::Patience),
            skew_tolerance: Some(10),
            unimportant: vec![UnimportantItem::Whitespace],
            ..TextRulesPatch::default()
        }),
        folder_rules_file: Some("/r/folder.toml".into()),
        criteria: Some("size,timestamp".into()),
        folder_rules: Some(FolderRulesPatch {
            size: Some(true),
            name_filter: Some("*.rs".into()),
            ..FolderRulesPatch::default()
        }),
        extra,
    }
}

fn every_request() -> Vec<Request> {
    vec![
        Request::Hello {
            client: "hikasa 0.1.0".into(),
            version: 1,
            min_version: 1,
        },
        Request::OpenView {
            kind: ViewKind::FolderCompare,
            specs: vec!["/a".into(), "/b".into()],
            options: sample_options(),
            wait: true,
        },
        Request::LoadSession {
            name: "s".into(),
            wait: true,
        },
        Request::LoadWorkspace { name: "w".into() },
        Request::RunScript {
            path: "/s.txt".into(),
            args: vec!["x".into()],
        },
        Request::ListViews,
        Request::CloseView { view: ViewId(3) },
        Request::Ping,
        Request::Shutdown,
    ]
}

fn every_response() -> Vec<Response> {
    vec![
        Response::Hello {
            server: "hikasa-gui 0.1.0".into(),
            version: 1,
        },
        Response::ViewOpened { view: ViewId(7) },
        Response::ViewClosed {
            view: ViewId(7),
            result: ViewResult::new(
                Some(ContentKind::ImportantDifferences),
                None,
                true,
                CloseReason::Shutdown,
            ),
            exit_code: 13,
        },
        Response::Views {
            views: vec![ViewInfo {
                view: ViewId(1),
                kind: ViewKind::TextCompare,
                title: "a.c ↔ b.c".into(),
                specs: vec!["/a.c".into(), "/b.c".into()],
            }],
        },
        Response::Ok,
        Response::Pong,
        Response::error(IpcErrorCode::NotFound, "no view 9"),
    ]
}

#[test]
fn every_message_round_trips() {
    for (k, request) in every_request().into_iter().enumerate() {
        let envelope = Envelope {
            id: k as u64 + 1,
            body: request,
        };
        let mut wire = Vec::new();
        send(&mut wire, &envelope).unwrap();
        let back = recv_request(&mut Cursor::new(wire)).unwrap().unwrap();
        assert_eq!(back, envelope);
    }
    for (k, response) in every_response().into_iter().enumerate() {
        let envelope = Envelope {
            id: k as u64 + 1,
            body: response,
        };
        let mut wire = Vec::new();
        send(&mut wire, &envelope).unwrap();
        let back = recv_response(&mut Cursor::new(wire)).unwrap().unwrap();
        assert_eq!(back, envelope);
    }
}

#[test]
fn wire_examples_of_4_4_3_are_golden() {
    let messages: Vec<String> = vec![
        to_line(
            1,
            &Request::Hello {
                client: "hikasa 0.1.0".into(),
                version: 1,
                min_version: 1,
            },
        ),
        to_line(
            1,
            &Response::Hello {
                server: "hikasa-gui 0.1.0".into(),
                version: 1,
            },
        ),
        to_line(
            2,
            &Request::OpenView {
                kind: ViewKind::TextCompare,
                specs: vec!["/w/a.c".into(), "/w/b.c".into()],
                options: OpenOptions::default(),
                wait: true,
            },
        ),
        to_line(2, &Response::ViewOpened { view: ViewId(7) }),
        to_line(
            2,
            &Response::ViewClosed {
                view: ViewId(7),
                result: ViewResult::new(
                    Some(ContentKind::UnimportantOnly),
                    None,
                    false,
                    CloseReason::User,
                ),
                exit_code: 12,
            },
        ),
        to_line(3, &Request::Ping),
        to_line(3, &Response::Pong),
        // A request type of a newer peer: written by hand, as an older build never sends one.
        r#"{"id":4,"type":"open_session_v9","name":"x"}"#.to_owned(),
        to_line(
            4,
            &Response::error(
                IpcErrorCode::UnsupportedRequest,
                "unknown request type `open_session_v9`",
            ),
        ),
    ];
    let actual = messages.join("\n") + "\n";
    common::assert_golden(
        &common::fixtures("ipc").join("wire_examples.jsonl"),
        actual.as_bytes(),
    );
    // Every golden line decodes to its typed value again (two-phase decode).
    let golden =
        std::fs::read_to_string(common::fixtures("ipc").join("wire_examples.jsonl")).unwrap();
    for (line, expected) in golden.lines().zip(&messages) {
        assert_eq!(line, expected);
    }
    let unknown = decode_request_line(golden.lines().nth(7).unwrap());
    assert_eq!(unknown.id, 4);
    assert_eq!(unknown.body, Request::Unknown);
}

fn to_line<T: serde::Serialize>(id: u64, body: &T) -> String {
    serde_json::to_string(&Envelope { id, body }).unwrap()
}

fn frame_of(json: &str) -> Vec<u8> {
    framed(&[json.as_bytes().to_vec()])
}

fn decode_request_line(json: &str) -> Envelope<Request> {
    recv_request(&mut Cursor::new(frame_of(json)))
        .unwrap()
        .unwrap()
}

#[test]
fn forward_compatible_decoding() {
    // Unknown fields are ignored (envelope, body, options, result).
    let request = decode_request_line(
        r#"{"id":9,"type":"open_view","kind":"text_compare","specs":["/a","/b"],"priority":3,
            "options":{"titles":["x",null,null,null],"future_option":true},"wait":false}"#,
    );
    assert_eq!(request.id, 9);
    match request.body {
        Request::OpenView {
            kind,
            options,
            wait,
            ..
        } => {
            assert_eq!(kind, ViewKind::TextCompare);
            assert!(!wait);
            assert_eq!(options.titles[0].as_deref(), Some("x"));
        }
        other => panic!("{other:?}"),
    }
    // Missing `options` and `wait` default.
    let minimal =
        decode_request_line(r#"{"id":1,"type":"open_view","kind":"folder_compare","specs":[]}"#);
    assert!(
        matches!(minimal.body, Request::OpenView { wait: false, ref options, .. } if *options == OpenOptions::default())
    );
    // An unknown view kind decodes to ViewKind::Unknown.
    let newer_kind = decode_request_line(
        r#"{"id":5,"type":"open_view","kind":"spreadsheet_compare","specs":[]}"#,
    );
    assert!(matches!(
        newer_kind.body,
        Request::OpenView {
            kind: ViewKind::Unknown,
            ..
        }
    ));
    // Unknown response types decode to Response::Unknown; view results keep unknown strings.
    let response = recv_response(&mut Cursor::new(frame_of(
        r#"{"id":2,"type":"progress_v2","pct":5}"#,
    )))
    .unwrap()
    .unwrap();
    assert_eq!(response.body, Response::Unknown);
    let closed = recv_response(&mut Cursor::new(frame_of(
        r#"{"id":2,"type":"view_closed","view":1,"result":{"verdict":"mostly_same","reason":"crashed","x":1},"exit_code":12}"#,
    )))
    .unwrap()
    .unwrap();
    match closed.body {
        Response::ViewClosed {
            result, exit_code, ..
        } => {
            assert_eq!(exit_code, 12);
            assert_eq!(result.verdict.as_deref(), Some("mostly_same"));
            assert_eq!(result.verdict_kind(), None);
            assert_eq!(result.close_reason(), None);
            assert!(!result.saved);
        }
        other => panic!("{other:?}"),
    }
    // Error codes this build does not know are kept.
    let error = recv_response(&mut Cursor::new(frame_of(
        r#"{"id":3,"type":"error","code":"quota_exceeded","message":"m"}"#,
    )))
    .unwrap()
    .unwrap();
    assert_eq!(
        error.body,
        Response::Error {
            code: "quota_exceeded".into(),
            message: "m".into()
        }
    );
    assert_eq!(IpcErrorCode::parse("quota_exceeded"), None);
}

#[test]
fn two_phase_decode_errors() {
    // A known type whose fields do not decode: BadMessage with the frame's own id.
    let bad = recv_request(&mut Cursor::new(frame_of(
        r#"{"id":12,"type":"close_view","view":"seven"}"#,
    )));
    assert!(
        matches!(bad, Err(IpcError::BadMessage { id: 12, .. })),
        "{bad:?}"
    );
    let missing_type = recv_request(&mut Cursor::new(frame_of(r#"{"id":13,"view":1}"#)));
    assert!(matches!(
        missing_type,
        Err(IpcError::BadMessage { id: 13, .. })
    ));
    // No usable id: Protocol.
    for json in [
        r#"{"type":"ping"}"#,
        r#"{"id":"1","type":"ping"}"#,
        r#"{"id":-1,"type":"ping"}"#,
        r#"[1,2]"#,
        r#"not json"#,
    ] {
        let err = recv_request(&mut Cursor::new(frame_of(json)));
        assert!(matches!(err, Err(IpcError::Protocol(_))), "{json}: {err:?}");
    }
    let invalid_utf8 = framed(&[vec![b'{', 0xff, b'}']]);
    assert!(matches!(
        recv_request(&mut Cursor::new(invalid_utf8)),
        Err(IpcError::Protocol(_))
    ));
}

#[test]
fn view_results_and_codes() {
    let result = ViewResult::new(
        Some(ContentKind::RulesSame),
        Some(CompareVerdict::Differences),
        true,
        CloseReason::LoadFailed,
    );
    assert_eq!(result.verdict.as_deref(), Some("rules_same"));
    assert_eq!(result.verdict_kind(), Some(ContentKind::RulesSame));
    assert_eq!(
        result.folder_verdict_kind(),
        Some(CompareVerdict::Differences)
    );
    assert_eq!(result.close_reason(), Some(CloseReason::LoadFailed));
    assert_eq!(result.reason, "load_failed");
    for code in IpcErrorCode::ALL {
        assert_eq!(IpcErrorCode::parse(code.as_str()), Some(code));
    }
    assert_eq!(
        IpcErrorCode::UnsupportedRequest.as_str(),
        "unsupported_request"
    );
    let info = ServerInfo {
        server: "s".into(),
        version: 1,
    };
    assert_eq!(
        serde_json::to_string(&info).unwrap(),
        r#"{"server":"s","version":1}"#
    );
    assert_eq!(Request::Ping.type_name(), "ping");
    assert_eq!(serde_json::to_string(&ViewId(4)).unwrap(), "4");
}
