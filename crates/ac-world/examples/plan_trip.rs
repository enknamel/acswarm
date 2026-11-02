//! Print the journey from one place to another.
//! `cargo run -p ac-world --example plan_trip -- FROM TO [level] [far]`, where
//! TO is a town name and FROM a town name or `ns,ew` map coordinates.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let from_xy = match a[0].split_once(',') {
        Some((ns, ew)) => ac_world::towns::world_xy(
            ns.trim().parse().expect("ns"),
            ew.trim().parse().expect("ew"),
        ),
        None => ac_world::towns::find(&a[0]).expect("from").world_xy(),
    };
    let to = ac_world::towns::find(&a[1]).expect("to");
    let level: u32 = a.get(2).and_then(|l| l.parse().ok()).unwrap_or(0);
    let t0 = std::time::Instant::now();
    let far = a.iter().any(|x| x == "far");
    let prefs = if far {
        ac_world::trip::Prefs::far()
    } else {
        ac_world::trip::Prefs::quick()
    };
    let (from, goal) = (
        ac_world::Place::on_map(from_xy),
        ac_world::Place::on_map(to.world_xy()),
    );
    let trip = ac_world::trip::plan_with(from, goal, level, &[], &[], prefs);
    println!(
        "{:?} -> {} at level {level}, planned in {:?}",
        a[0],
        to.name,
        t0.elapsed()
    );
    let map = |p: &ac_world::Place| ac_world::towns::map_of(p.at.truncate());
    match trip {
        Some(t) => {
            println!("{} ({:.0} s)", t.summary(), t.seconds);
            for s in &t.steps {
                match s {
                    ac_world::trip::Step::Walk(p) => {
                        println!("  walk to {}", map(p).0)
                    }
                    ac_world::trip::Step::Gem { name, exit, .. } => {
                        println!("  use {name:?} -> {:?}", map(exit))
                    }
                    ac_world::trip::Step::Portal {
                        name, mouth, exit, ..
                    } => println!("  portal {name:?} at {:?} -> {:?}", map(mouth), map(exit)),
                    ac_world::trip::Step::Recall { name, exit, .. } => {
                        println!("  cast {name:?} -> {:?}", map(exit))
                    }
                }
            }
        }
        None => println!("no trip"),
    }
}
