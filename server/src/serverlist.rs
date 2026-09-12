use crate::http::Response;
use serde_json::Value;
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
///
/// `location` is a one-time, hand-resolved lookup at add-time, not live
/// geolocation -- `server.xindeler.com` resolves to `216.238.126.97`, hosted
/// in São Paulo, Brazil (verified via `dig` + a GeoIP lookup, not guessed).
/// If this server ever moves hosts, update this constant by hand; there is
/// no automatic re-resolution.
fn official_servers() -> Vec<GameServer> {
    vec![GameServer::new(
        "Xindeler",
        "server.xindeler.com",
        14004,
        Some(14006),
        "The official Xindeler server.",
        country_parser::parse("BR"),
        "https://auth.xindeler.com",
        Some("release"),
        true,
        Default::default(),
    )]
}

/// `veloren-serverbrowser-api` 0.4.0's `location` field is asymmetric: it
/// serializes an absent location as JSON `null` (`serialize_none()`), but
/// its own deserializer calls `String::deserialize` directly on that same
/// field, which cannot handle `null` -- any client using this crate to
/// parse our response panics on the whole list, not just that field, the
/// moment a server omits its location. Confirmed directly by reading the
/// crate's `serialize_country`/`deserialize_country` functions, not just
/// asserted. Since this is a bug in the crate itself, not our data, the fix
/// lives here: drop `location` entirely from the JSON when it's `null`
/// rather than relying on the crate's own (broken) round-trip.
fn strip_null_location(mut list: Value) -> Value {
    if let Some(servers) = list.get_mut("servers").and_then(Value::as_array_mut) {
        for server in servers {
            if let Some(map) = server.as_object_mut() {
                if map.get("location").is_some_and(Value::is_null) {
                    map.remove("location");
                }
            }
        }
    }
    list
}

pub fn list_servers(_request: &crate::http::Request) -> Response {
    let list = GameServerList {
        servers: official_servers(),
    };
    match serde_json::to_value(&list) {
        Ok(value) => Response::json(&strip_null_location(value)),
        Err(_) => Response::json(&list),
    }
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
        assert_eq!(
            server.location.as_ref().map(|c| c.iso2.as_str()),
            Some("BR")
        );
    }

    #[test]
    fn strip_null_location_removes_a_null_location_field() {
        let list = serde_json::json!({
            "servers": [{"name": "Xindeler", "location": null}]
        });
        let stripped = strip_null_location(list);
        assert!(stripped["servers"][0].get("location").is_none());
    }

    #[test]
    fn strip_null_location_leaves_a_real_location_untouched() {
        let list = serde_json::json!({
            "servers": [{"name": "Xindeler", "location": "US"}]
        });
        let stripped = strip_null_location(list);
        assert_eq!(stripped["servers"][0]["location"], "US");
    }

    #[test]
    fn the_real_response_never_contains_a_null_location() {
        // Regression test for the actual bug: veloren-serverbrowser-api
        // 0.4.0 serializes an absent location as JSON `null`, but its own
        // deserializer can't parse that `null` back -- any client using
        // this crate to parse our response would panic on the whole list.
        let list = GameServerList {
            servers: official_servers(),
        };
        let value = strip_null_location(serde_json::to_value(&list).unwrap());
        for server in value["servers"].as_array().unwrap() {
            assert!(
                server.get("location").is_none_or(|l| !l.is_null()),
                "a server response must never contain an explicit `location: null` -- \
                 veloren-serverbrowser-api 0.4.0's client can't deserialize that"
            );
        }
    }
}
