//! Tier 2 and 3: a configured parser, and the pull reader picking a JSON-RPC
//! envelope apart without building a tree, keeping `params` as raw text.
//!
//! Run with `cargo run --example rpc_envelope`.

use jsonx_lib::{DuplicateKeys, Event, Limits, Parser};

/// Messages from a client: small, shallow, and no duplicate keys.
const WIRE: Parser = Parser::new()
    .limits(
        Limits::new()
            .with_max_input_bytes(1 << 20)
            .with_max_depth(32),
    )
    .duplicate_keys(DuplicateKeys::Error);

fn main() -> Result<(), jsonx_lib::Error> {
    let body = r#"{"jsonrpc": "2.0", "id": 7, "method": "textDocument/hover",
                   "params": {"textDocument": {"uri": "file:///a.rs"}, "position": {"line": 3, "character": 9}}}"#;

    let mut reader = WIRE.reader(body);
    let (mut id, mut method, mut params) = (None, None, None);
    if reader.next_event()? == Some(Event::StartObject) {
        while let Some(Event::Key(key)) = reader.next_event()? {
            match &*key {
                "id" => id = reader.read_raw()?,
                "method" => method = reader.read_value()?,
                "params" => params = reader.read_raw()?,
                _ => {
                    let _skipped = reader.read_raw()?;
                }
            }
        }
    }
    reader.finish()?;

    println!("id     = {id:?}");
    println!("method = {:?}", method.as_ref().and_then(|m| m.as_str()));
    println!("params = {params:?}");

    // Later, the handler parses just its parameters.
    if let Some(raw) = params {
        let p = WIRE.parse(raw)?;
        println!("line   = {:?}", p["position"]["line"].as_u64());
    }
    Ok(())
}
