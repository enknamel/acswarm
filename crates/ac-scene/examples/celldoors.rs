//! A cell's doorways and the cells behind its portals.
use ac_scene::{landblock, Assets};
fn main() {
    let dir = std::env::var_os("AC_DATA_DIR").expect("AC_DATA_DIR");
    let assets = Assets::open(std::path::Path::new(&dir)).unwrap();
    let mut args = std::env::args().skip(1);
    let block =
        u32::from_str_radix(args.next().unwrap().trim_start_matches("0x"), 16).unwrap() << 16;
    let scene = landblock::load(&assets, block).unwrap();
    for a in args {
        let id = u32::from_str_radix(a.trim_start_matches("0x"), 16).unwrap();
        let Some(c) = scene.cells.iter().find(|c| c.cell_id == id) else {
            println!("{id:#010x}: no such cell");
            continue;
        };
        let o = c.transform.transform_point3(glam::Vec3::ZERO);
        println!(
            "{id:#010x} env {:#x} structure {} origin ({:.1},{:.1},{:.1}) portals {:x?}",
            c.environment_id, c.cell_structure, o.x, o.y, o.z, c.portal_cells
        );
        for d in &c.doorways {
            println!("    doorway ({:.2},{:.2},{:.2})", d.x, d.y, d.z);
        }
    }
}
