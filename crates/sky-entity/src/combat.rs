/// Computes horizontal knockback displacement `[dx, dy, dz]` and hurt yaw angle when an attacker
/// at `attacker_pos` strikes `victim_pos`.
pub fn compute_melee_knockback(
    attacker_pos: [f64; 3],
    victim_pos: [f64; 3],
    strength: f64,
) -> ([f64; 3], f32) {
    let dx = victim_pos[0] - attacker_pos[0];
    let dz = victim_pos[2] - attacker_pos[2];
    let len = (dx * dx + dz * dz).sqrt();
    if len < 1e-6 {
        return ([0.0, 0.36, strength], 0.0);
    }
    let nx = dx / len;
    let nz = dz / len;
    let hurt_yaw_deg = (nz.atan2(nx).to_degrees() as f32) - 90.0;
    ([nx * strength, 0.36, nz * strength], hurt_yaw_deg)
}
