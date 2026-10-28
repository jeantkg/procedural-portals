//! Componentizes the core module and embeds the host's requested-grants custom section.

const GRANTS_SECTION: &str = "cinnabar:grants";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.len() != 2 {
        return Err("usage: pack CORE.wasm COMPONENT.wasm".into());
    }
    let source = std::path::Path::new(&arguments[0]);
    let destination = std::path::Path::new(&arguments[1]);
    if source == destination {
        return Err("input and output must differ".into());
    }
    let mut component = wit_component::ComponentEncoder::default()
        .module(&std::fs::read(source)?)?
        .validate(true)
        .encode()?;
    let grants = include_str!("../grants.json").trim();
    let mut section = Vec::new();
    leb128(GRANTS_SECTION.len(), &mut section);
    section.extend_from_slice(GRANTS_SECTION.as_bytes());
    section.extend_from_slice(grants.as_bytes());
    component.push(0);
    leb128(section.len(), &mut component);
    component.extend(section);
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&component)?;
    if let wit_component::DecodedWasm::Component(resolve, world) =
        wit_component::decode(&component)?
    {
        for key in resolve.worlds[world].imports.keys() {
            let name = resolve.name_world_key(key);
            if name.contains("local-terrain") {
                return Err("cached build must not import the proposed local-terrain API".into());
            }
            println!("Import: {name}");
        }
    } else {
        return Err("expected an executable component".into());
    }
    std::fs::write(destination, component)?;
    Ok(())
}

/// Encodes the unsigned section length without assuming a one-byte payload.
fn leb128(mut value: usize, output: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        output.push(if value == 0 { byte } else { byte | 0x80 });
        if value == 0 {
            return;
        }
    }
}
