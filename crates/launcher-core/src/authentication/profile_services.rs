//! Skin and cape calls of the Minecraft Services profile API, and the public texture host.
//!
//! These calls reuse the bounded, nonredirecting transport of the parent module and the
//! access token the session already holds, so the token never leaves this crate. Service
//! answers are reduced to plain values here; their bodies never become errors.
use std::time::Instant;

use reqwest::header::{CACHE_CONTROL, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::{AuthClient, MinecraftSession, Stage};
use crate::error::{CoreError, ErrorCode};
use crate::skins::SkinModel;

/// Largest texture the public host may send. A high-resolution skin is far smaller.
const MAX_TEXTURE: usize = 1024 * 1024;
const TEXTURE_HOST: &str = "textures.minecraft.net";
const MAX_HASH: usize = 128;
const MAX_ALIAS_CHARS: usize = 64;

/// The skin and capes of one profile as the service reports them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemoteProfile {
    pub id: Uuid,
    pub name: String,
    /// The active skin; `None` when the profile has none of its own.
    pub skin: Option<RemoteSkin>,
    pub capes: Vec<RemoteCape>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemoteSkin {
    /// The hash in the texture host's address, never the whole URL.
    pub texture: String,
    pub model: SkinModel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemoteCape {
    pub id: String,
    pub alias: String,
    pub texture: String,
    pub active: bool,
}

#[derive(Deserialize)]
struct ProfileDto {
    id: String,
    name: String,
    #[serde(default)]
    skins: Vec<SkinDto>,
    #[serde(default)]
    capes: Vec<CapeDto>,
}

#[derive(Deserialize)]
struct SkinDto {
    #[serde(default)]
    state: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    variant: String,
}

#[derive(Deserialize)]
struct CapeDto {
    id: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    alias: String,
}

/// The hash of a texture address, or `None` when the address is not exactly a texture of
/// the public host. Anything else is ignored, so a tampered answer cannot make the
/// launcher fetch from another host.
pub(crate) fn texture_hash(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str() != Some(TEXTURE_HOST)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    let mut segments = parsed.path_segments()?;
    let (Some("texture"), Some(hash), None) = (segments.next(), segments.next(), segments.next())
    else {
        return None;
    };
    valid_hash(hash).then(|| hash.to_ascii_lowercase())
}

/// A texture hash of the public host: hexadecimal and bounded.
pub(crate) fn valid_hash(hash: &str) -> bool {
    !hash.is_empty() && hash.len() <= MAX_HASH && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Cape identifiers name a file in the cape cache, so only a plain token qualifies.
pub(crate) fn valid_cape_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn clean_alias(alias: &str, fallback: &str) -> String {
    let cleaned: String = alias
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_ALIAS_CHARS)
        .collect();
    if cleaned.trim().is_empty() {
        fallback.to_owned()
    } else {
        cleaned.trim().to_owned()
    }
}

impl ProfileDto {
    fn into_remote(self, session: &MinecraftSession) -> Result<RemoteProfile, CoreError> {
        let id = Uuid::parse_str(&self.id).map_err(|_| CoreError::new(ErrorCode::InvalidData))?;
        if id != session.profile.id {
            return Err(CoreError::new(ErrorCode::AccountMismatch));
        }
        let skin = self
            .skins
            .iter()
            .filter(|skin| skin.state.eq_ignore_ascii_case("active"))
            .find_map(|skin| {
                Some(RemoteSkin {
                    texture: texture_hash(&skin.url)?,
                    model: if skin.variant.eq_ignore_ascii_case("slim") {
                        SkinModel::Slim
                    } else {
                        SkinModel::Classic
                    },
                })
            });
        let capes = self
            .capes
            .iter()
            .filter(|cape| valid_cape_id(&cape.id))
            .filter_map(|cape| {
                Some(RemoteCape {
                    id: cape.id.clone(),
                    alias: clean_alias(&cape.alias, "Capa"),
                    texture: texture_hash(&cape.url)?,
                    active: cape.state.eq_ignore_ascii_case("active"),
                })
            })
            .collect();
        Ok(RemoteProfile {
            id,
            name: self.name,
            skin,
            capes,
        })
    }
}

/// The `multipart/form-data` body of a skin upload: the model and the PNG file.
fn skin_form(boundary: &str, model: SkinModel, png: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(png.len() + 512);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"variant\"\r\n\r\n{}\r\n",
            model.service_name()
        )
        .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"skin.png\"\r\nContent-Type: image/png\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(png);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

impl AuthClient {
    /// The profile with its skins and capes. Always a remote request.
    pub(crate) async fn service_profile(
        &self,
        session: &MinecraftSession,
    ) -> Result<RemoteProfile, CoreError> {
        let dto: ProfileDto = self
            .request(
                self.client
                    .get(&self.endpoints.profile)
                    .bearer_auth(session.access_token().expose())
                    .header(CACHE_CONTROL, "no-store"),
                Stage::Profile,
            )
            .await?;
        dto.into_remote(session)
    }

    /// Replace the active skin with a PNG. The service answers with the new profile.
    pub(crate) async fn upload_skin(
        &self,
        session: &MinecraftSession,
        model: SkinModel,
        png: &[u8],
    ) -> Result<RemoteProfile, CoreError> {
        let boundary = format!("panita-{}", Uuid::new_v4().simple());
        let dto: ProfileDto = self
            .request(
                self.client
                    .post(format!("{}/skins", self.endpoints.profile))
                    .bearer_auth(session.access_token().expose())
                    .header(
                        CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(skin_form(&boundary, model, png)),
                Stage::SkinUpload,
            )
            .await?;
        dto.into_remote(session)
    }

    /// Show one of the profile's capes, or none.
    pub(crate) async fn choose_cape(
        &self,
        session: &MinecraftSession,
        cape: Option<&str>,
    ) -> Result<RemoteProfile, CoreError> {
        let url = format!("{}/capes/active", self.endpoints.profile);
        let access = session.access_token().expose();
        let dto: ProfileDto = match cape {
            Some(id) => {
                if !valid_cape_id(id) {
                    return Err(CoreError::new(ErrorCode::InvalidData));
                }
                self.request(
                    self.client
                        .put(url)
                        .bearer_auth(access)
                        .json(&json!({ "capeId": id })),
                    Stage::CapeSet,
                )
                .await?
            }
            None => {
                self.request(self.client.delete(url).bearer_auth(access), Stage::CapeHide)
                    .await?
            }
        };
        dto.into_remote(session)
    }

    /// A skin or cape image from the public texture host. It needs no credential and is
    /// bounded in size; the caller validates it as an image before keeping it.
    pub(crate) async fn download_texture(&self, hash: &str) -> Result<Vec<u8>, CoreError> {
        if !valid_hash(hash) {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        let started = Instant::now();
        let sent = self
            .client
            .get(format!("{}/texture/{hash}", self.endpoints.textures))
            .send()
            .await;
        crate::network_log::api_result(
            "textures",
            "GET",
            "/texture",
            started,
            sent.as_ref().map(|reply| reply.status().as_u16()),
            false,
        );
        let mut response = sent.map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?;
        let status = response.status();
        if !status.is_success() {
            return Err(CoreError::new(
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                    || status.is_server_error()
                    || status.is_redirection()
                {
                    ErrorCode::ServiceUnavailable
                } else {
                    ErrorCode::InvalidData
                },
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_TEXTURE as u64)
        {
            return Err(CoreError::new(ErrorCode::InvalidData));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| CoreError::new(ErrorCode::NetworkUnavailable))?
        {
            if bytes.len() + chunk.len() > MAX_TEXTURE {
                return Err(CoreError::new(ErrorCode::InvalidData));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::super::Endpoints;
    use super::*;
    use crate::accounts::Secret;
    use crate::authentication::JavaProfile;
    use crate::test_http::{Reply, Server};

    const ID: &str = "5627dd98e6be3c21b8a8e92344183641";

    fn client(server: &Server) -> AuthClient {
        let base = server.base();
        AuthClient {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_else(|_| panic!("client")),
            endpoints: Endpoints {
                microsoft: format!("{base}/token"),
                xbox: format!("{base}/xbox"),
                xsts: format!("{base}/xsts"),
                minecraft: format!("{base}/minecraft"),
                profile: format!("{base}/profile"),
                textures: format!("{base}/textures"),
            },
        }
    }

    fn session() -> MinecraftSession {
        MinecraftSession {
            profile: JavaProfile {
                id: Uuid::parse_str(ID).unwrap_or_else(|_| panic!("id")),
                name: "PlayerOne".to_owned(),
            },
            access: Secret::new("fixture-token".to_owned()).unwrap_or_else(|_| panic!("secret")),
            expires_at: Instant::now() + Duration::from_secs(3600),
            expires_wall: SystemTime::now() + Duration::from_secs(3600),
        }
    }

    fn profile_json(id: &str) -> String {
        json!({
            "id": id, "name": "PlayerOne",
            "skins": [
                {"id":"s1","state":"INACTIVE","url":"http://textures.minecraft.net/texture/aaaa","variant":"CLASSIC"},
                {"id":"s2","state":"ACTIVE","url":"http://textures.minecraft.net/texture/BBBB","variant":"SLIM"}
            ],
            "capes": [
                {"id":"11111111-1111-4111-8111-111111111111","state":"ACTIVE","url":"http://textures.minecraft.net/texture/cccc","alias":"Migrator"},
                {"id":"22222222-2222-4222-8222-222222222222","state":"INACTIVE","url":"http://evil.test/texture/dddd","alias":"Foreign host"},
                {"id":"../bad","state":"INACTIVE","url":"http://textures.minecraft.net/texture/eeee","alias":"Bad id"}
            ]
        })
        .to_string()
    }

    #[tokio::test]
    async fn the_profile_is_reduced_to_the_active_skin_and_trusted_capes() {
        let server = Server::start(|_| Reply::ok(profile_json(ID)));
        let remote = client(&server)
            .service_profile(&session())
            .await
            .unwrap_or_else(|_| panic!("profile"));
        let skin = remote.skin.unwrap_or_else(|| panic!("skin"));
        assert_eq!(
            (skin.texture.as_str(), skin.model),
            ("bbbb", SkinModel::Slim)
        );
        // A cape from another host and one with an unsafe identifier are dropped.
        assert_eq!(remote.capes.len(), 1);
        assert_eq!(remote.capes[0].alias, "Migrator");
        assert!(remote.capes[0].active);
        let request = &server.requests()[0];
        assert!(request.headers.starts_with("get /profile "));
        assert!(
            request
                .headers
                .contains("authorization: bearer fixture-token")
        );
        assert!(request.headers.contains("cache-control: no-store"));
    }

    #[tokio::test]
    async fn a_profile_without_skins_is_a_profile_without_a_custom_skin() {
        let server = Server::start(|_| Reply::ok(format!(r#"{{"id":"{ID}","name":"PlayerOne"}}"#)));
        let remote = client(&server)
            .service_profile(&session())
            .await
            .unwrap_or_else(|_| panic!("profile"));
        assert_eq!(remote.skin, None);
        assert!(remote.capes.is_empty());
    }

    #[tokio::test]
    async fn another_profile_is_never_accepted() {
        let other = "11111111111141118111111111111111";
        let server = Server::start(move |_| Reply::ok(profile_json(other)));
        let error = client(&server).service_profile(&session()).await.err();
        assert_eq!(error.map(|e| e.code()), Some(ErrorCode::AccountMismatch));
    }

    #[tokio::test]
    async fn uploads_and_cape_changes_use_the_documented_routes() {
        let server = Server::start(|_| Reply::ok(profile_json(ID)));
        let client = client(&server);
        let session = session();
        client
            .upload_skin(&session, SkinModel::Slim, b"png")
            .await
            .unwrap_or_else(|_| panic!("upload"));
        client
            .choose_cape(&session, Some("11111111-1111-4111-8111-111111111111"))
            .await
            .unwrap_or_else(|_| panic!("set"));
        client
            .choose_cape(&session, None)
            .await
            .unwrap_or_else(|_| panic!("hide"));
        let requests = server.requests();
        assert!(requests[0].headers.starts_with("post /profile/skins "));
        assert!(
            requests[0]
                .headers
                .contains("content-type: multipart/form-data; boundary=panita-")
        );
        assert!(
            requests[1]
                .headers
                .starts_with("put /profile/capes/active ")
        );
        assert!(
            requests[1]
                .headers
                .contains("content-type: application/json")
        );
        assert!(
            requests[2]
                .headers
                .starts_with("delete /profile/capes/active ")
        );
        assert!(
            requests
                .iter()
                .all(|r| r.headers.contains("authorization: bearer fixture-token"))
        );
        // An unsafe identifier never reaches the network.
        assert!(client.choose_cape(&session, Some("../x")).await.is_err());
        assert_eq!(server.requests().len(), 3);
    }

    #[tokio::test]
    async fn failures_are_classified_without_exposing_the_service_body() {
        for (status, expected) in [
            (401, ErrorCode::AuthenticationRequired),
            (403, ErrorCode::AccountRestricted),
            (429, ErrorCode::ServiceUnavailable),
            (503, ErrorCode::ServiceUnavailable),
        ] {
            let server = Server::start(move |_| Reply::status(status).body("sensitive fixture"));
            let error = client(&server)
                .choose_cape(&session(), None)
                .await
                .err()
                .unwrap_or_else(|| panic!("rejected"));
            assert_eq!(error.code(), expected, "{status}");
            assert!(!format!("{error:?}").contains("sensitive"));
        }
        let server = Server::start(|_| Reply::status(400).body("{}"));
        let error = client(&server)
            .upload_skin(&session(), SkinModel::Classic, b"png")
            .await
            .err();
        assert_eq!(error.map(|e| e.code()), Some(ErrorCode::InvalidSkin));
    }

    #[tokio::test]
    async fn textures_are_fetched_from_the_texture_host_and_bounded() {
        let server = Server::start(|request| {
            if request.path == "/textures/texture/abc123" {
                Reply::ok(vec![7_u8; 100])
            } else if request.path == "/textures/texture/ffff" {
                Reply::ok(vec![0_u8; MAX_TEXTURE + 1])
            } else {
                Reply::status(404)
            }
        });
        let client = client(&server);
        let bytes = client
            .download_texture("abc123")
            .await
            .unwrap_or_else(|_| panic!("texture"));
        assert_eq!(bytes.len(), 100);
        assert!(!server.requests()[0].headers.contains("authorization"));
        assert_eq!(
            client
                .download_texture("ffff")
                .await
                .err()
                .map(|e| e.code()),
            Some(ErrorCode::InvalidData)
        );
        assert_eq!(
            client
                .download_texture("0000")
                .await
                .err()
                .map(|e| e.code()),
            Some(ErrorCode::InvalidData)
        );
        let before = server.requests().len();
        for bad in ["", "../x", "zz", &"a".repeat(129)] {
            assert!(client.download_texture(bad).await.is_err(), "{bad}");
        }
        assert_eq!(server.requests().len(), before);
    }

    #[test]
    fn only_textures_of_the_public_host_are_accepted() {
        let good = "http://textures.minecraft.net/texture/ABCDEF0123456789";
        assert_eq!(texture_hash(good).as_deref(), Some("abcdef0123456789"));
        assert_eq!(
            texture_hash("https://textures.minecraft.net/texture/abc123").as_deref(),
            Some("abc123")
        );
        for bad in [
            "https://textures.minecraft.net.evil.test/texture/abc",
            "https://evil.test/texture/abc",
            "https://user@textures.minecraft.net/texture/abc",
            "https://textures.minecraft.net/texture/abc/extra",
            "https://textures.minecraft.net/other/abc",
            "https://textures.minecraft.net/texture/not-hex!",
            "https://textures.minecraft.net/texture/",
            "ftp://textures.minecraft.net/texture/abc",
            "not a url",
            "",
        ] {
            assert_eq!(texture_hash(bad), None, "{bad}");
        }
        let long = format!("https://textures.minecraft.net/texture/{}", "a".repeat(129));
        assert_eq!(texture_hash(&long), None);
    }

    #[test]
    fn cape_identifiers_are_plain_tokens() {
        assert!(valid_cape_id("5af2dc8d-8d0e-4d9e-a2a0-1b5cf3d1cf12"));
        for bad in ["", "../x", "a/b", "a b", "a.png", &"a".repeat(65)] {
            assert!(!valid_cape_id(bad), "{bad}");
        }
    }

    #[test]
    fn the_upload_form_carries_the_model_and_the_exact_file_bytes() {
        let png = [0x89, b'P', b'N', b'G', 0, 1, 2, 255];
        let body = skin_form("edge", SkinModel::Slim, &png);
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with(
            "--edge\r\nContent-Disposition: form-data; name=\"variant\"\r\n\r\nslim\r\n--edge\r\n"
        ));
        assert!(
            text.contains(
                "name=\"file\"; filename=\"skin.png\"\r\nContent-Type: image/png\r\n\r\n"
            )
        );
        assert!(text.ends_with("\r\n--edge--\r\n"));
        assert!(body.windows(png.len()).any(|window| window == png));
        let classic = skin_form("edge", SkinModel::Classic, &png);
        assert!(String::from_utf8_lossy(&classic).contains("\r\n\r\nclassic\r\n"));
    }

    #[test]
    fn aliases_lose_control_characters_and_never_end_up_empty() {
        assert_eq!(clean_alias("Mi\u{7} capa ", "Capa"), "Mi capa");
        assert_eq!(clean_alias("   ", "Capa"), "Capa");
        assert_eq!(clean_alias(&"x".repeat(200), "Capa").chars().count(), 64);
    }
}
