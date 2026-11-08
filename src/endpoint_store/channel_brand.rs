// src/endpoint_store/channel_brand.rs
//
// A workspace's own branding for the app a messaging platform shows: today the
// Teams app, whose package the dashboard builds in the browser. A software
// vendor puts its product in its customers' Teams, so the app should carry the
// vendor's logo, colour and legal links rather than api0's.
//
// Stored as one JSON object in `messaging_channels.app_brand`. Registering the
// bot again keeps it; removing the bot removes it. Every field is optional:
// what is missing falls back to api0's own in the package.
//
// The icons are PNGs, sent base64-encoded. Teams rejects a package whose icons
// are not exactly 192×192 (colour) and 32×32 (outline), so the sizes are
// checked here too, from the PNG header, and not only in the dashboard.

use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};
use base64::Engine;
use serde::{Deserialize, Serialize};

/// Largest icon accepted, decoded. Teams' own limit is far above what a
/// 192×192 PNG needs; this keeps a stray photo out of the database.
pub const MAX_ICON_BYTES: usize = 100 * 1024;
const MAX_URL_CHARS: usize = 2048;
pub const COLOR_SIZE: u32 = 192;
pub const OUTLINE_SIZE: u32 = 32;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AppBrand {
    /// 192×192 PNG, base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_png: Option<String>,
    /// 32×32 PNG, white on transparent, base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_png: Option<String>,
    /// "#RRGGBB": the tile Teams puts behind the outline icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms_url: Option<String>,
}

/// Empty strings mean "not set", so a cleared field in the form clears it here.
fn blank_to_none(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Width and height from a PNG's IHDR chunk, or why the bytes are not a PNG.
pub fn png_size(bytes: &[u8]) -> Result<(u32, u32), &'static str> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return Err("is not a PNG image");
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Ok((w, h))
}

fn check_icon(name: &str, b64: &str, size: u32) -> Result<(), String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| format!("The {} icon could not be read", name))?;
    if bytes.len() > MAX_ICON_BYTES {
        return Err(format!("The {} icon is larger than {} KB", name, MAX_ICON_BYTES / 1024));
    }
    let (w, h) = png_size(&bytes).map_err(|why| format!("The {} icon {}", name, why))?;
    if (w, h) != (size, size) {
        return Err(format!("The {} icon must be {}×{} pixels, not {}×{}", name, size, size, w, h));
    }
    Ok(())
}

fn check_url(name: &str, url: &str) -> Result<(), String> {
    if url.chars().count() > MAX_URL_CHARS {
        return Err(format!("The {} link is too long", name));
    }
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| format!("The {} link must start with https://", name))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty() || !host.contains('.') || url.chars().any(char::is_whitespace) {
        return Err(format!("The {} link is not a web address", name));
    }
    Ok(())
}

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

impl AppBrand {
    /// Trim, drop empty fields and check what is left. The error is written for
    /// the person filling in the form.
    pub fn validated(self) -> Result<AppBrand, String> {
        let b = AppBrand {
            color_png: blank_to_none(self.color_png),
            outline_png: blank_to_none(self.outline_png),
            accent_color: blank_to_none(self.accent_color).map(|c| c.to_ascii_uppercase()),
            website_url: blank_to_none(self.website_url),
            privacy_url: blank_to_none(self.privacy_url),
            terms_url: blank_to_none(self.terms_url),
        };
        // One without the other would put the vendor's logo next to api0's.
        if b.color_png.is_some() != b.outline_png.is_some() {
            return Err("Add both icons, the colour one and the outline one".into());
        }
        if let Some(c) = &b.color_png {
            check_icon("colour", c, COLOR_SIZE)?;
        }
        if let Some(o) = &b.outline_png {
            check_icon("outline", o, OUTLINE_SIZE)?;
        }
        if let Some(a) = &b.accent_color {
            if !is_hex_color(a) {
                return Err("The colour must look like #1A73E8".into());
            }
        }
        for (name, url) in [("website", &b.website_url), ("privacy", &b.privacy_url), ("terms", &b.terms_url)] {
            if let Some(u) = url {
                check_url(name, u)?;
            }
        }
        Ok(b)
    }
}

/// The brand of a workspace's channel. `None` when the workspace has no such
/// channel; an empty brand when it has one with nothing set.
pub async fn get_channel_brand(
    store: &EndpointStore,
    tenant_id: &str,
    channel: &str,
) -> Result<Option<AppBrand>, StoreError> {
    let client = store.get_admin_conn().await?;
    let row = client
        .query_opt(
            "SELECT app_brand FROM messaging_channels WHERE tenant_id = $1 AND channel = $2",
            &[&tenant_id, &channel],
        )
        .await
        .to_store_error()?;
    Ok(row.map(|r| {
        let v: serde_json::Value = r.get(0);
        serde_json::from_value(v).unwrap_or_default()
    }))
}

/// Replace the brand of a workspace's channel. `false` when it has no such channel.
pub async fn set_channel_brand(
    store: &EndpointStore,
    tenant_id: &str,
    channel: &str,
    brand: &AppBrand,
) -> Result<bool, StoreError> {
    let value = serde_json::to_value(brand).map_err(|e| StoreError::InvalidInput(e.to_string()))?;
    let client = store.get_admin_conn().await?;
    let updated = client
        .execute(
            "UPDATE messaging_channels SET app_brand = $3 WHERE tenant_id = $1 AND channel = $2",
            &[&tenant_id, &channel, &value],
        )
        .await
        .to_store_error()?;
    Ok(updated > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest bytes `png_size` accepts: signature, then an IHDR header.
    fn png(w: u32, h: u32) -> String {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]);
        base64::engine::general_purpose::STANDARD.encode(b)
    }

    fn with_icons() -> AppBrand {
        AppBrand { color_png: Some(png(192, 192)), outline_png: Some(png(32, 32)), ..Default::default() }
    }

    #[test]
    fn an_empty_brand_is_valid_and_stays_empty() {
        let b = AppBrand { accent_color: Some("  ".into()), website_url: Some(String::new()), ..Default::default() };
        assert_eq!(b.validated().unwrap(), AppBrand::default());
    }

    #[test]
    fn icons_of_the_right_size_pass() {
        assert!(with_icons().validated().is_ok());
    }

    #[test]
    fn an_icon_of_the_wrong_size_is_refused_with_its_size() {
        let b = AppBrand { color_png: Some(png(512, 512)), ..with_icons() };
        let e = b.validated().unwrap_err();
        assert!(e.contains("192×192") && e.contains("512×512"), "{}", e);
    }

    #[test]
    fn one_icon_without_the_other_is_refused() {
        let b = AppBrand { outline_png: None, ..with_icons() };
        assert!(b.validated().is_err());
    }

    #[test]
    fn something_that_is_not_a_png_is_refused() {
        let jpeg = base64::engine::general_purpose::STANDARD.encode([0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F']);
        let b = AppBrand { color_png: Some(jpeg), ..with_icons() };
        assert!(b.validated().unwrap_err().contains("not a PNG"));
    }

    #[test]
    fn colours_must_be_hex_and_are_upper_cased() {
        let ok = AppBrand { accent_color: Some("#1a73e8".into()), ..Default::default() }.validated().unwrap();
        assert_eq!(ok.accent_color.as_deref(), Some("#1A73E8"));
        assert!(AppBrand { accent_color: Some("blue".into()), ..Default::default() }.validated().is_err());
    }

    #[test]
    fn links_must_be_https_web_addresses() {
        let ok = AppBrand { privacy_url: Some("https://acme.ch/privacy".into()), ..Default::default() };
        assert!(ok.validated().is_ok());
        for bad in ["http://acme.ch/privacy", "https://", "https://localhost/x", "javascript:alert(1)", "https://acme.ch/a b"] {
            let b = AppBrand { privacy_url: Some(bad.into()), ..Default::default() };
            assert!(b.validated().is_err(), "{} should be refused", bad);
        }
    }
}
