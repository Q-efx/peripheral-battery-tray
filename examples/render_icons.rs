//! Writes sample tray icons as raw RGBA files (for previewing the design).
use battery_tray::icon::{render, IconState, SIZE};
fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let samples = [
        ("a_67", Some(67), false, true),
        ("b_100_chg", Some(100), true, true),
        ("c_35", Some(35), false, true),
        ("d_8", Some(8), false, true),
        ("e_asleep_54", Some(54), false, false),
        ("f_none", None, false, true),
        ("g_unanswered", None, false, false),
    ];
    for (name, level, charging, live) in samples {
        let unanswered = level.is_none() && !live;
        let px = render(&IconState { level, charging, live: live || unanswered, unanswered });
        std::fs::write(format!("{dir}/{name}.rgba"), px).unwrap();
    }
    println!("{SIZE}");
}
