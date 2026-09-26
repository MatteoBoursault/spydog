use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;
use spydog::{config_path, data_dir, expand_env};

#[derive(Deserialize)]
struct Event {
  ts: u64,
  cwd: String,
  source: String,
  #[serde(rename = "type")]
  kind: String,
  #[serde(default)]
  payload: Option<serde_json::Value>,
}

#[derive(Deserialize, Clone)]
#[serde(default)]
struct Config {
  db_path: Option<PathBuf>,
  batch_size: usize,
  flush_interval_ms: u64,
}

impl Default for Config {
  fn default() -> Self {
    Config {
      db_path: None,
      batch_size: 100,
      flush_interval_ms: 1000,
    }
  }
}

fn load_config() -> Config {
  let path = config_path();
  if let Ok(s) = std::fs::read_to_string(&path) {
    return toml::from_str(&s).unwrap_or_default();
  }
  let cfg = Config::default();
  if let Some(parent) = path.parent() {
    let _ = std::fs::create_dir_all(parent);
  }
  let default_db = data_dir().join("events.db");
  let content = format!(
    "# spydog — configuration du daemon\n\
     db_path = \"{}\"\n\
     batch_size = {}\n\
     flush_interval_ms = {}\n",
    default_db.display(),
    cfg.batch_size,
    cfg.flush_interval_ms
  );
  let _ = std::fs::write(&path, content);
  cfg
}

fn git_info(cwd: &str) -> (Option<String>, Option<String>) {
  let run = |args: &[&str]| -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
      return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
      None
    } else {
      Some(s)
    }
  };
  (
    run(&["-C", cwd, "rev-parse", "--show-toplevel"]),
    run(&["-C", cwd, "branch", "--show-current"]),
  )
}

fn init_schema(conn: &rusqlite::Connection) {
  conn
    .execute_batch(
      "CREATE TABLE IF NOT EXISTS events (
            timestamp INTEGER NOT NULL,
            source TEXT NOT NULL,
            type TEXT NOT NULL,
            payload TEXT,
            cwd TEXT,
            project TEXT,
            branch TEXT,
            session_id TEXT
        );",
    )
    .expect("init schema");
}

fn flush(
  conn: &mut rusqlite::Connection,
  batch: &mut Vec<Event>,
  cache: &mut Option<(String, Option<String>, Option<String>)>,
  session_id: &str,
) {
  let tx = conn.transaction().expect("begin");
  {
    let mut stmt = tx
      .prepare(
        "INSERT INTO events (timestamp, source, type, payload, cwd, project, branch, session_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
      )
      .expect("prepare");
    for ev in batch.drain(..) {
      let (project, branch) = match cache {
        Some((cwd, p, b)) if *cwd == ev.cwd => (p.clone(), b.clone()),
        _ => {
          let (p, b) = git_info(&ev.cwd);
          *cache = Some((ev.cwd.clone(), p.clone(), b.clone()));
          (p, b)
        }
      };
      let payload = ev.payload.as_ref().map(|p| p.to_string());
      stmt
        .execute(rusqlite::params![
          ev.ts as i64,
          ev.source,
          ev.kind,
          payload,
          ev.cwd,
          project,
          branch,
          session_id
        ])
        .ok();
    }
  }
  tx.commit().ok();
}

fn main() {
  let cfg = load_config();
  let dir = data_dir();
  let _ = std::fs::create_dir_all(&dir);
  // Données potentiellement sensibles (commandes, collages).
  std::fs::set_permissions(&dir, PermissionsExt::from_mode(0o700)).ok();

  let sock_path = dir.join("spydog.sock");
  let lock_path = dir.join("spydog.lock");
  let db_path = cfg
    .db_path
    .as_ref()
    .map(|p| PathBuf::from(expand_env(&p.to_string_lossy())))
    .unwrap_or_else(|| dir.join("events.db"));
  // La config peut placer la base hors du data dir (ex. home hôte).
  if let Some(parent) = db_path.parent() {
    let _ = std::fs::create_dir_all(parent);
    std::fs::set_permissions(parent, PermissionsExt::from_mode(0o700)).ok();
  }

  // Anti-doublon : verrou exclusif auto-libéré à la mort du processus (flock).
  let lock = std::fs::File::create(&lock_path).expect("lock file");
  if lock.try_lock().is_err() {
    std::process::exit(0); // déjà en route → un seul daemon
  }

  // Le lock garantit l'exclusivité : une socket restante appartient à une
  // instance morte → suppression sans risque.
  let _ = std::fs::remove_file(&sock_path);
  let sock = UnixDatagram::bind(&sock_path).expect("bind socket");
  std::fs::set_permissions(&sock_path, PermissionsExt::from_mode(0o600)).ok();
  sock
    .set_read_timeout(Some(Duration::from_millis(cfg.flush_interval_ms)))
    .ok();

  let session_id = format!("{:x}", spydog::epoch_ms());
  let mut conn = rusqlite::Connection::open(&db_path).expect("open db");
  std::fs::set_permissions(&db_path, PermissionsExt::from_mode(0o600)).ok();
  init_schema(&conn);

  let mut batch: Vec<Event> = Vec::new();
  let mut cache: Option<(String, Option<String>, Option<String>)> = None;
  // datagram max ~200 Ko ; un collage plus gros est perdu côté client (send_to échoue)
  let mut buf = vec![0u8; 262_144];

  loop {
    match sock.recv_from(&mut buf) {
      Ok((n, _)) => {
        if let Ok(ev) = serde_json::from_slice::<Event>(&buf[..n]) {
          batch.push(ev);
          if batch.len() >= cfg.batch_size {
            flush(&mut conn, &mut batch, &mut cache, &session_id);
          }
        }
      }
      Err(e)
        if e.kind() == std::io::ErrorKind::WouldBlock
          || e.kind() == std::io::ErrorKind::TimedOut =>
      {
        if !batch.is_empty() {
          flush(&mut conn, &mut batch, &mut cache, &session_id);
        }
      }
      Err(_) => std::thread::sleep(Duration::from_millis(10)),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn insert_roundtrip() {
    let mut conn = rusqlite::Connection::open_in_memory().unwrap();
    init_schema(&conn);
    let mut batch = vec![Event {
      ts: 123,
      cwd: "/tmp".into(),
      source: "fish".into(),
      kind: "command".into(),
      payload: Some(serde_json::json!({"cmd": "ls"})),
    }];
    let mut cache = None;
    flush(&mut conn, &mut batch, &mut cache, "sess1");
    let n: i64 = conn
      .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
      .unwrap();
    assert_eq!(n, 1);
  }

  #[test]
  fn config_defaults_and_parse() {
    let c: Config = toml::from_str("").unwrap();
    assert_eq!(c.batch_size, 100);
    assert_eq!(c.flush_interval_ms, 1000);
    assert!(c.db_path.is_none());

    let c: Config =
      toml::from_str("batch_size = 5\nflush_interval_ms = 250\ndb_path = \"/tmp/x.db\"").unwrap();
    assert_eq!(c.batch_size, 5);
    assert_eq!(c.flush_interval_ms, 250);
    assert_eq!(c.db_path.unwrap(), PathBuf::from("/tmp/x.db"));
  }
}
