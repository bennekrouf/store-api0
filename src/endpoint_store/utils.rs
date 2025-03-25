use slug::slugify;
use uuid::Uuid;

/// Generates a random UUID string
pub fn generate_uuid() -> String {
    Uuid::new_v4().to_string()
}

/// Generate ID from text using slugify and UUID for uniqueness
pub fn generate_id_from_text(text: &str) -> String {
    let slug = slugify(text);
    if slug.is_empty() {
        return generate_uuid();
    }

    // Create a shorter UUID suffix (first 8 chars)
    let uuid_short = Uuid::new_v4()
        .to_string()
        .split('-')
        .next()
        .unwrap_or("")
        .to_string();
    format!("{}-{}", slug, uuid_short)
}
