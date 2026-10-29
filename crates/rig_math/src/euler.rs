//! Quaternion <-> Euler conversion for formats that animate Euler angles (FBX).

/// Converts a quaternion `(x, y, z, w)` to `XYZ` Euler angles in degrees.
///
/// `XYZ` applies X first, i.e. `R = Rz * Ry * Rx` (FBX `eEulerXYZ`). The input
/// is normalized first because spline-decoded quaternions are not unit length
/// between control points, and the formulas below assume they are.
pub fn quat_to_euler_xyz([mut x, mut y, mut z, mut w]: [f64; 4]) -> [f64; 3] {
    let length = w.mul_add(w, z.mul_add(z, y.mul_add(y, x * x))).sqrt();
    if length <= f64::EPSILON {
        return [0.0; 3];
    }
    x /= length;
    y /= length;
    z /= length;
    w /= length;

    let roll = (2.0 * y.mul_add(z, w * x)).atan2(2.0f64.mul_add(-y.mul_add(y, x * x), 1.0));
    // Clamped: rounding can push |sin| slightly above 1 near gimbal lock.
    let pitch = (2.0 * z.mul_add(-x, w * y)).clamp(-1.0, 1.0).asin();
    let yaw = (2.0 * x.mul_add(y, w * z)).atan2(2.0f64.mul_add(-z.mul_add(z, y * y), 1.0));

    [roll.to_degrees(), pitch.to_degrees(), yaw.to_degrees()]
}

/// Picks the `XYZ` Euler representation of `euler` closest to `prev`.
///
/// Curves are interpolated per component, so a jump such as 179° -> -179°
/// spins a bone almost a full turn between two keys. Both solutions of the
/// same rotation, `(x, y, z)` and `(x + 180, 180 - y, z + 180)`, are unwrapped
/// by multiples of 360° toward `prev`, and the nearer one wins.
pub fn compatible_euler(euler: [f64; 3], prev: [f64; 3]) -> [f64; 3] {
    fn unwrap(e: [f64; 3], prev: [f64; 3]) -> [f64; 3] {
        core::array::from_fn(|i| ((prev[i] - e[i]) / 360.0).round().mul_add(360.0, e[i]))
    }

    fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
        a.iter().zip(&b).map(|(a, b)| (a - b).abs()).sum()
    }

    let direct = unwrap(euler, prev);
    let flipped = unwrap([euler[0] + 180.0, 180.0 - euler[1], euler[2] + 180.0], prev);

    if distance(flipped, prev) < distance(direct, prev) {
        flipped
    } else {
        direct
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mat4;

    #[test]
    fn euler_matches_matrix() {
        let euler = [30.0, -45.0, 120.0];
        let (_, q, _) = Mat4::from_euler_degrees(euler, 0).decompose();
        let back = quat_to_euler_xyz(q);
        for (a, b) in euler.iter().zip(back) {
            assert!((a - b).abs() < 1.0e-9, "{euler:?} != {back:?}");
        }
    }

    #[test]
    fn compatible_euler_unwraps() {
        assert_eq!(
            compatible_euler([-179.0, 0.0, 0.0], [179.0, 0.0, 0.0]),
            [181.0, 0.0, 0.0]
        );
    }
}
