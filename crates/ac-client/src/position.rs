//! Where the character stands.

use crate::Client;

impl Client {
    /// The player's world position (landblock origin plus local), None until placed.
    pub fn my_position(&self) -> Option<glam::Vec3> {
        self.player.as_ref().map(|p| p.world_position())
    }
}

#[cfg(test)]
mod tests {
    use crate::Client;

    #[test]
    fn my_position_is_none_until_placed_then_in_world_coordinates() {
        let Some(dir) = std::env::var_os("AC_DATA_DIR") else {
            eprintln!("AC_DATA_DIR unset; skipping");
            return;
        };
        let assets = std::rc::Rc::new(ac_scene::Assets::open(dir).unwrap());
        // Offline: nothing calls `tick`, so no packet is ever sent.
        let mut c = Client::connect(
            crate::Config {
                host: "127.0.0.1:1".into(),
                account: "acreborn".into(),
                password: "x".into(),
                character: None,
                auto_enter: true,
            },
            assets.clone(),
        )
        .unwrap();
        assert_eq!(c.my_position(), None);

        const HOLTBURG: u32 = 0xA9B4_0019;
        let local = glam::vec3(84.0, 108.0, 94.0);
        c.player = Some(crate::player::Player::new(
            &assets,
            HOLTBURG,
            local,
            glam::Quat::IDENTITY,
        ));
        assert_eq!(
            c.my_position(),
            Some(ac_world::landblock_origin(HOLTBURG) + local)
        );
    }
}
