//! The program a case's `pane` steps run in, the same on every platform
//! (`tmxr_compat_oracle::PaneStep`): `oracle-pane <steps.toml>`.

use std::io::{Read, Write};
use std::time::Duration;

use tmxr_compat_oracle::{PaneStep, PaneSteps};

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .expect("usage: oracle-pane <steps.toml>");
    let text = std::fs::read_to_string(&path).expect("read the steps");
    let PaneSteps { steps } = toml::from_str(&text).expect("parse the steps");
    let mut out = std::io::stdout().lock();
    for step in steps {
        match step {
            PaneStep::Print(s) => {
                out.write_all(s.as_bytes()).unwrap();
                out.flush().unwrap();
            }
            PaneStep::Seq(n) => {
                for i in 1..=n {
                    writeln!(out, "{i}").unwrap();
                }
                out.flush().unwrap();
            }
            PaneStep::Sleep(secs) => std::thread::sleep(Duration::from_secs_f64(secs)),
            PaneStep::Raw => raw(),
            PaneStep::Cat => cat(&mut out),
        }
    }
    loop {
        std::thread::park();
    }
}

/// Input raw and unechoed, as `stty raw -echo`. On Windows also the
/// console's VT input (ttyr's raw mode turns on VT output): ConPTY hands
/// the program the escape sequences it is sent, and the mouse modes it asks
/// for reach the pane, only with it.
fn raw() {
    ttyr::terminal::enable_raw_mode().expect("raw mode");
    #[cfg(windows)]
    ttyr::terminal::enable(
        &mut std::io::stdout(),
        ttyr::terminal::Mode::VirtualTerminalInput,
    )
    .expect("VT input");
}

/// Write all input as `cat -v` does, for ever: a control byte as `^X`
/// (but newline and tab as they are), DEL as `^?`, a byte with the high
/// bit set as `M-` and the rest.
fn cat(out: &mut impl Write) -> ! {
    let mut input = std::io::stdin().lock();
    let mut buf = [0; 4096];
    loop {
        let n = input.read(&mut buf).expect("read input");
        if n == 0 {
            loop {
                std::thread::park();
            }
        }
        let mut text = Vec::with_capacity(n * 2);
        for &b in &buf[..n] {
            let b = if b >= 0x80 {
                text.extend_from_slice(b"M-");
                b - 0x80
            } else {
                b
            };
            match b {
                b'\n' | b'\t' => text.push(b),
                0..0x20 => text.extend_from_slice(&[b'^', b + 0x40]),
                0x7f => text.extend_from_slice(b"^?"),
                _ => text.push(b),
            }
        }
        out.write_all(&text).unwrap();
        out.flush().unwrap();
    }
}
