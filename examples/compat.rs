//! Compare a new component's imports against an already working component.
use std::collections::HashSet;
use wit_component::{DecodedWasm, decode};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: compat WORKING.wasm CANDIDATE.wasm".into());
    }
    let DecodedWasm::Component(mut baseline, old_world) = decode(&std::fs::read(&args[0])?)? else {
        return Err("working file is not a component".into());
    };
    let DecodedWasm::Component(candidate, new_world) = decode(&std::fs::read(&args[1])?)? else {
        return Err("candidate file is not a component".into());
    };
    let old: HashSet<_> = baseline.worlds[old_world]
        .imports
        .keys()
        .map(|key| baseline.name_world_key(key))
        .collect();
    for key in candidate.worlds[new_world].imports.keys() {
        let name = candidate.name_world_key(key);
        if !old.contains(&name) {
            return Err(format!("new import not present in working component: {name}").into());
        }
    }
    // Shared package/interface/type definitions must agree, not merely have matching names.
    baseline.merge(candidate)?;
    println!(
        "PASS: candidate imports are a subset of the working component; shared WIT definitions agree."
    );
    Ok(())
}
