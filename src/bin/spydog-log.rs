use std::os::unix::net::UnixDatagram;

use spydog::{data_dir, epoch_ms};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: spydog-log <source> <type> [payload-json]");
        std::process::exit(1);
    }
    let source = &args[1];
    let kind = &args[2];
    let payload = args.get(3).and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());

    let event = serde_json::json!({
        "ts": epoch_ms(),
        "cwd": std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default(),
        "source": source,
        "type": kind,
        "payload": payload,
    });

    let sock = match UnixDatagram::unbound() {
        Ok(s) => s,
        Err(_) => std::process::exit(1),
    };
    let _ = sock.send_to(event.to_string().as_bytes(), data_dir().join("spydog.sock"));
}
