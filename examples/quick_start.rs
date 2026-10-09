//! The Tier-1 path: parse, read, build, write.
//!
//! Run with `cargo run --example quick_start`.

use jsonx_lib::{json, parse, to_string, to_string_pretty};

fn main() -> Result<(), jsonx_lib::Error> {
    // Parse: strings without escapes and all numbers borrow from the text.
    let text = r#"{"name": "jsonx", "version": [0, 2, 0], "tags": ["json", "no-std"]}"#;
    let doc = parse(text)?;

    // Read: indexing never panics; a missing member reads as null.
    println!("name    = {}", doc["name"]);
    println!("minor   = {:?}", doc["version"][1].as_u64());
    println!("missing = {}", doc["license"]);

    // Build with `json!`, write compact or pretty.
    let reply = json!({
        "ok": true,
        "name": doc["name"].clone(),
        "tag_count": doc["tags"].as_array().map_or(0, |tags| tags.len()),
    });
    println!("{}", to_string(&reply));
    println!("{}", to_string_pretty(&reply));

    // Errors name the kind and the exact byte.
    if let Err(e) = parse(r#"{"a": [1, 2,]}"#) {
        println!("refused: {e}");
    }
    Ok(())
}
