use crate::http::Response;
use veloren_serverbrowser_api::{GameServer, GameServerList};

/// A hand-maintained static directory, not a database table or a
/// self-registration system — matches upstream Veloren's own
/// `gitlab.com/veloren/serverbrowser` model (a curated `servers.ron`, third
/// parties get listed by filing a request, not by self-registering). This
/// crate's own `GameServer` schema carries no live fields at all (no player
/// count, no version) — that data comes from the game's own `query_server`
/// UDP protocol, requested by the client directly against each server's own
/// `query_port`, never routed through this endpoint.
///
/// Add an entry here by hand for each additional officially-listed server;
/// third-party listing requests go through the GitHub issue link
/// `xindeler-updater` already points players at.
fn official_servers() -> Vec<GameServer> {
    vec![GameServer::new(
        "Xindeler",
        "server.xindeler.com",
        14004,
        Some(14006),
        "The official Xindeler server.",
        None,
        "https://auth.xindeler.com",
        Some("release"),
        true,
        Default::default(),
    )]
}

pub fn list_servers(_request: &crate::http::Request) -> Response {
    Response::json(&GameServerList {
        servers: official_servers(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_servers_lists_exactly_the_real_game_server() {
        let servers = official_servers();
        assert_eq!(servers.len(), 1);
        let server = &servers[0];
        assert_eq!(server.address, "server.xindeler.com");
        assert_eq!(server.port, 14004);
        // The game's own query_server UDP protocol port (server-cli's
        // default) -- distinct from `port` above, which is the TCP game
        // protocol port.
        assert_eq!(server.query_port, Some(14006));
        assert_eq!(server.auth_server, "https://auth.xindeler.com");
        assert!(server.official);
    }
}
