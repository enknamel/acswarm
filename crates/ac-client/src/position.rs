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
    use crate::testkit;

    #[test]
    fn my_position_is_none_until_placed_then_in_world_coordinates() {
        let mut c = testkit::offline_client();
        assert_eq!(c.my_position(), None);

        const HOLTBURG: u32 = 0xA9B4_0019;
        let local = glam::vec3(84.0, 108.0, 94.0);
        testkit::stand(&mut c, HOLTBURG, local);
        assert_eq!(
            c.my_position(),
            Some(ac_world::landblock_origin(HOLTBURG) + local)
        );
    }
}
