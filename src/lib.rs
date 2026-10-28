mod cache;
mod config;
mod live;
mod live_scene;
mod portals;
mod runtime;
mod scenery;
mod view;

use mod_api::PlayerMod;
use std::cell::RefCell;

thread_local! { static RUNTIME: RefCell<runtime::Runtime> = RefCell::new(runtime::Runtime::default()); }

struct ProceduralPortals;
impl PlayerMod for ProceduralPortals {
    fn init() {
        RUNTIME.with(|runtime| runtime.borrow_mut().init());
    }
    fn frame() {
        RUNTIME.with(|runtime| runtime.borrow_mut().frame());
    }
}
mod_api::export_player_mod!(ProceduralPortals);

#[cfg(test)]
mod tests {
    #[test]
    fn declared_permissions_are_limited_to_native_rendering_observations_and_status() {
        let grants: serde_json::Value =
            serde_json::from_str(include_str!("../grants.json")).unwrap();
        for name in [
            "player_state",
            "terrain",
            "settings",
            "world_draw",
            "render_material",
            "hud",
        ] {
            assert_eq!(grants[name], true);
        }
        assert_eq!(grants.as_object().unwrap().len(), 6);
    }
}
