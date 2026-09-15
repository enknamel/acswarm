/// Body of FellowshipUpdateRequest (0x00A6): whether the fellowship
/// panel is open. ACE sends FellowshipUpdateFellow vitals (and a
/// FellowshipFullUpdate at once) only to members who reported it open,
/// so a client that reads fellows' health must send `true` after
/// joining.
pub fn fellowship_update_request(panel_open: bool) -> Vec<u8> {
    u32::from(panel_open).to_le_bytes().to_vec()
}
