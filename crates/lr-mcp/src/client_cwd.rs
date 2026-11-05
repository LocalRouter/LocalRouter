//! Best-effort launch-directory detection for MCP clients on this host.
//!
//! This is process context, never an authorization or filesystem access boundary.
use std::{net::SocketAddr, path::PathBuf};

pub const BRIDGE_CWD_HEADER: &str = "x-localrouter-client-cwd";

/// Only direct loopback peers can identify a process on this host. Forwarded
/// headers are deliberately ignored. Unsupported hosts retain the saved fallback.
pub async fn detect(peer: SocketAddr, server_port: u16) -> Option<PathBuf> {
    if !peer.ip().is_loopback() {
        return None;
    }
    #[cfg(unix)]
    {
        let output = tokio::process::Command::new(lsof_path())
            .args([
                "-nP",
                "-a",
                &format!("-iTCP:{}", peer.port()),
                "-sTCP:ESTABLISHED",
                "-Fpn",
            ])
            .kill_on_drop(true)
            .output()
            .await
            .ok()?;
        let sockets = String::from_utf8(output.stdout).ok()?;
        let pid = socket_owner(&sockets, peer, server_port)?;
        let output = tokio::process::Command::new(lsof_path())
            .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
            .kill_on_drop(true)
            .output()
            .await
            .ok()?;
        let cwd = String::from_utf8(output.stdout).ok()?;
        // Recheck the socket after reading cwd: the peer may have disconnected
        // or its PID/port may have been reused while the lookup was in flight.
        let output = tokio::process::Command::new(lsof_path())
            .args([
                "-nP",
                "-a",
                "-p",
                &pid.to_string(),
                &format!("-iTCP:{}", peer.port()),
                "-sTCP:ESTABLISHED",
                "-Fpn",
            ])
            .kill_on_drop(true)
            .output()
            .await
            .ok()?;
        if socket_owner(&String::from_utf8(output.stdout).ok()?, peer, server_port) != Some(pid) {
            return None;
        }
        cwd.lines()
            .find_map(|line| line.strip_prefix('n'))
            .and_then(valid_directory)
    }
    #[cfg(not(unix))]
    {
        let _ = server_port;
        None
    }
}

/// Decode context relayed by a local stdio bridge. Remote machines' paths
/// must never be interpreted as directories on this host.
pub fn bridge_directory(peer: SocketAddr, encoded: &str) -> Option<PathBuf> {
    if !peer.ip().is_loopback() {
        return None;
    }
    valid_directory(&urlencoding::decode(encoded).ok()?)
}

pub fn valid_directory(value: &str) -> Option<PathBuf> {
    let path = PathBuf::from(value);
    (path.is_absolute() && path.is_dir()).then_some(path)
}

#[cfg(unix)]
fn lsof_path() -> &'static str {
    if cfg!(target_os = "macos") {
        "/usr/sbin/lsof"
    } else {
        "lsof"
    }
}

#[cfg(any(unix, test))]
fn socket_owner(output: &str, peer: SocketAddr, server_port: u16) -> Option<u32> {
    let mut pid = None;
    let mut owner = None;
    for line in output.lines() {
        if let Some(value) = line.strip_prefix('p') {
            pid = value.parse().ok();
        } else if let Some((source, destination)) =
            line.strip_prefix('n').and_then(|s| s.split_once("->"))
        {
            if source.parse::<SocketAddr>().ok() == Some(peer)
                && destination
                    .parse::<SocketAddr>()
                    .is_ok_and(|a| a.ip().is_loopback() && a.port() == server_port)
            {
                let candidate = pid?;
                if owner.is_some_and(|owner| owner != candidate) {
                    return None;
                }
                owner = Some(candidate);
            }
        }
    }
    owner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_client_side_full_socket_tuple() {
        let sockets =
            "p10\nn127.0.0.1:3625->127.0.0.1:5555\np20\nn127.0.0.1:5555->127.0.0.1:3625\n";
        assert_eq!(
            socket_owner(sockets, "127.0.0.1:5555".parse().unwrap(), 3625),
            Some(20)
        );
        assert_eq!(
            socket_owner(sockets, "127.0.0.1:5555".parse().unwrap(), 33625),
            None
        );
        assert_eq!(
            socket_owner(sockets, "127.0.0.2:5555".parse().unwrap(), 3625),
            None
        );
    }

    #[test]
    fn supports_ipv6_and_rejects_ambiguous_owners() {
        let peer = "[::1]:5555".parse().unwrap();
        let sockets = "p20\nn[::1]:5555->[::1]:3625\n";
        assert_eq!(socket_owner(sockets, peer, 3625), Some(20));
        assert_eq!(
            socket_owner(
                &format!("{sockets}p21\nn[::1]:5555->[::1]:3625\n"),
                peer,
                3625
            ),
            None
        );
    }

    #[tokio::test]
    async fn remote_peers_cannot_resolve_local_processes() {
        assert_eq!(detect("192.0.2.1:5555".parse().unwrap(), 3625).await, None);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn resolves_a_live_loopback_socket_to_this_process_directory() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (_server, peer) = listener.accept().await.unwrap();
        let directory = detect(peer, listener.local_addr().unwrap().port())
            .await
            .unwrap();
        assert_eq!(
            directory.canonicalize().unwrap(),
            std::env::current_dir().unwrap().canonicalize().unwrap()
        );
        drop(client);
    }

    #[test]
    fn bridge_paths_are_decoded_and_remote_paths_are_ignored() {
        let directory = std::env::temp_dir().join(format!("lr cwd ü {}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let encoded = urlencoding::encode(directory.to_str().unwrap());
        assert_eq!(
            bridge_directory("127.0.0.1:5555".parse().unwrap(), &encoded),
            Some(directory.clone())
        );
        assert_eq!(
            bridge_directory("192.0.2.1:5555".parse().unwrap(), &encoded),
            None
        );
        assert_eq!(
            bridge_directory("127.0.0.1:5555".parse().unwrap(), "relative"),
            None
        );
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn directories_must_be_absolute_and_exist() {
        assert!(valid_directory(".").is_none());
        assert!(valid_directory("/does-not-exist-localrouter-cwd").is_none());
        assert_eq!(
            valid_directory(std::env::temp_dir().to_str().unwrap()),
            Some(std::env::temp_dir())
        );
    }
}
