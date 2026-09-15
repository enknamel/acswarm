//! Which points round a spot have a floor: a map of the room's shape.
use ac_scene::Assets;
use glam::Vec3;
fn main() {
    let dir = std::env::var_os("AC_DATA_DIR").expect("AC_DATA_DIR");
    let assets = Assets::open(std::path::Path::new(&dir)).unwrap();
    let a: Vec<f32> = std::env::args()
        .skip(2)
        .map(|s| s.parse().unwrap())
        .collect();
    let block = u32::from_str_radix(
        std::env::args().nth(1).unwrap().trim_start_matches("0x"),
        16,
    )
    .unwrap()
        << 16;
    let (cx, cy, z, r) = (a[0], a[1], a[2], a[3]);
    let coll = assets.block_collision(block).unwrap();
    let mut y = cy + r;
    while y >= cy - r {
        let mut row = String::new();
        let mut x = cx - r;
        while x <= cx + r {
            let f = coll.world.floor_at(Vec3::new(x, y, z), 0.6, 1.5);
            row.push(match f {
                Some((fz, _)) if (fz - z).abs() < 0.3 => '.',
                Some(_) => 'v',
                None => '#',
            });
            x += 0.5;
        }
        println!("{y:9.1} {row}");
        y -= 0.5;
    }
    println!(
        "          x from {} to {} step 0.5; . floor at z, v floor at another height, # none",
        cx - r,
        cx + r
    );
}
