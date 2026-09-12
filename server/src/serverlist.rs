use crate::http::Response;
use crate::state::AppState;
use country_parser::Country;
use serde_json::Value;
use std::net::ToSocketAddrs;
use std::time::Duration;
use veloren_serverbrowser_api::{GameServer, GameServerList};

const GEOIP_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const GEOIP_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

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
/// `xindeler-updater` already points players at. `location` resolves
/// automatically from `address` (see `official_location`) — never hardcode
/// a country code here, it would silently go stale if this server ever
/// moves hosts.
fn official_servers(location: Option<Country>) -> Vec<GameServer> {
    vec![GameServer::new(
        "Xindeler",
        "server.xindeler.com",
        14004,
        Some(14006),
        "The official Xindeler server.",
        location,
        "https://auth.xindeler.com",
        Some("release"),
        true,
        Default::default(),
    )]
}

/// Resolves a server's location from its own address — the same
/// one-time-lookup-at-add-time pattern `xindeler-updater` already validated
/// client-side for manually added servers (DNS -> IP -> GeoIP, no live
/// tracking, no API key). Never panics and never blocks the caller for long
/// — DNS resolution failure, an unreachable GeoIP service, or an
/// unparseable country code all just return `None`, which
/// `strip_null_location` below turns into an omitted field rather than a
/// broken `null`.
fn resolve_location(address: &str, geoip_base_url: &str) -> Option<Country> {
    let ip = (address, 0u16).to_socket_addrs().ok()?.next()?.ip();
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(GEOIP_CONNECT_TIMEOUT)
        .timeout(GEOIP_REQUEST_TIMEOUT)
        .build()
        .ok()?;
    let response = client.get(format!("{geoip_base_url}/{ip}")).send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: Value = response.json().ok()?;
    let code = body.get("country_code")?.as_str()?;
    country_parser::parse(code)
}

/// Resolved once per process lifetime, on first request into
/// `state.server_location_cache` — this data almost never changes (it only
/// would if the official server moved hosts, which already requires a code
/// change to `official_servers`'s hardcoded address anyway, and a restart
/// naturally re-resolves it then), so there's no need for a TTL.
fn official_location(state: &AppState) -> Option<Country> {
    state
        .server_location_cache
        .get_or_init(|| resolve_location("server.xindeler.com", &state.geoip_base_url))
        .clone()
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

pub fn list_servers(_request: &crate::http::Request, state: &AppState) -> Response {
    let list = GameServerList {
        servers: official_servers(official_location(state)),
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
        let servers = official_servers(country_parser::parse("BR"));
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
    fn resolve_location_returns_none_for_an_unresolvable_address() {
        // No DNS lookup for this hostname will ever succeed -- proves the
        // failure path returns `None` instead of panicking, without
        // depending on network access.
        assert_eq!(
            resolve_location("this-host-does-not-exist.invalid", "https://ipwho.is"),
            None
        );
    }

    #[test]
    fn resolve_location_returns_none_when_the_geoip_service_is_unreachable() {
        // Port 1 on loopback refuses connections instantly -- proves a
        // dead GeoIP service degrades to `None` rather than panicking or
        // hanging, without depending on network access or a live mock.
        assert_eq!(resolve_location("127.0.0.1", "http://127.0.0.1:1"), None);
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
        // Covers both outcomes of `official_location` (resolved or not),
        // without depending on live network access.
        for location in [country_parser::parse("BR"), None] {
            let list = GameServerList {
                servers: official_servers(location),
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
}
