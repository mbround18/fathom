//! 035-cloud-auth-plugins test double: a cross-platform stand-in for `aws eks get-token` /
//! `gke-gcloud-auth-plugin` / `kubelogin` that speaks the ExecCredential protocol.
//!
//! Usage: `fake-exec-plugin <mode> <counter-file> [lifetime-secs]`
//! Modes: `ok` (token, expires in `lifetime-secs`, default 900), `noexpiry`, `fail` (stderr
//! echoes the token, exit 1), `login` (stderr says the SSO session expired, exit 1), `hang`
//! (sleeps), `expired` (returns an already-expired credential), `garbage` (not JSON).
//! Every invocation appends one byte to the counter file so tests can count spawns. The token is
//! `$FAKE_TOKEN` or a fixed `k8s-aws-v1.` shaped value.

use std::io::Write;

const DEFAULT_TOKEN: &str = "k8s-aws-v1.RkFLRVRPS0VOMTIzNDU2Nzg5MEFCQ0RFRkdISUpLTE1O";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map_or("ok", String::as_str);
    if let Some(path) = args.get(2)
        && let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        let _ = f.write_all(b"x");
    }
    let lifetime: i64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(900);
    let token = std::env::var("FAKE_TOKEN").unwrap_or_else(|_| DEFAULT_TOKEN.to_owned());
    let cred = |exp: Option<i64>| {
        let mut status = serde_json::json!({ "token": token });
        if let Some(secs) = exp {
            status["expirationTimestamp"] = serde_json::Value::String(
                (chrono::Utc::now() + chrono::Duration::seconds(secs)).to_rfc3339(),
            );
        }
        println!(
            "{}",
            serde_json::json!({
                "apiVersion": "client.authentication.k8s.io/v1beta1",
                "kind": "ExecCredential",
                "status": status,
            })
        );
    };
    match mode {
        "ok" => cred(Some(lifetime)),
        "noexpiry" => cred(None),
        "expired" => cred(Some(-3600)),
        "fail" => {
            eprintln!("fake plugin failure, token was {token}");
            std::process::exit(1);
        }
        "login" => {
            eprintln!(
                "The SSO session associated with this profile has expired. To refresh this SSO session run aws sso login"
            );
            std::process::exit(255);
        }
        "hang" => std::thread::sleep(std::time::Duration::from_secs(3600)),
        "garbage" => println!("this is not json"),
        other => {
            eprintln!("unknown mode {other}");
            std::process::exit(2);
        }
    }
}
