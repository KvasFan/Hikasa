#![allow(dead_code)]
fn f<'a>(x: &'a str) -> char { let c = 'x'; let e = '\''; let u = '\u{1F600}'; c }
let raw = r#"a "quoted" \n"#; let b = b"bytes\x00"; let cs = c"c str";
let n = 1_000u32 + 0xFF_u8 as u32 + 1e-3f64 as u32; let r#type = 2.5f32;
/* block /* nested */ still comment? */ let s = "multi
line";
let größe = 'static_lifetime_is_not_a_char;
