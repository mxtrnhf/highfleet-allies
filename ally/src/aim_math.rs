//! Mean cannon-shell trajectory, not a promise that a random-spread round will hit.
//! Steam 1.163: BulletImpact 0x140052a80, FireShell 0x1400300d0 and
//! b2Island_Solve use a 10-unit muzzle offset, 0.98..1.2 speed multiplier,
//! gravity 9.82 and linear damping 0.05 at a 0.02-second physics step.
//! Only unguided, zero-quadratic-drag shells are supported by this linear model.

const STEP: f32 = 0.02;
const DAMPING: f32 = 0.05;
const GRAVITY: f32 = 9.82;
const MUZZLE_OFFSET: f32 = 10.0;
const MAX_STEPS: usize = 1500;

pub struct Solution {
    pub direction: [f32; 2],
    pub distance: f32,
    pub flight: f32,
}

/// Invert the discrete damped/gravity trajectory and find its first reachable intercept.
/// `relative_position` is target centre minus gun origin; target velocity is world-space,
/// and `inherited_velocity` is the firing gun's native master-root velocity, not the player's hull's.
/// The target is assumed to keep its current velocity. No native memory is accessed here.
pub fn solve(
    relative_position: [f32; 2],
    target_velocity: [f32; 2],
    inherited_velocity: [f32; 2],
    speed: f32,
    lifespan: f32,
) -> Option<Solution> {
    if !relative_position
        .iter()
        .chain(target_velocity.iter())
        .chain(inherited_velocity.iter())
        .all(|v| v.is_finite())
    {
        return None;
    }
    if !speed.is_finite() || speed <= 0.0 || !lifespan.is_finite() || lifespan <= 0.0 {
        return None;
    }

    let mut initial_velocity_factor = 1.0;
    let mut displacement_factor = 0.0;
    let mut gravity_velocity = 0.0;
    let mut gravity_displacement = 0.0;
    let mut previous_error = relative_position[0].hypot(relative_position[1]) - MUZZLE_OFFSET;
    if previous_error <= 0.0 {
        return None;
    }

    for step in 1..=MAX_STEPS {
        let time = step as f32 * STEP;
        if time > lifespan {
            break;
        }
        let previous_displacement_factor = displacement_factor;
        let previous_gravity_displacement = gravity_displacement;
        initial_velocity_factor *= 1.0 - STEP * DAMPING;
        displacement_factor += initial_velocity_factor * STEP;
        gravity_velocity = (gravity_velocity + GRAVITY * STEP) * (1.0 - STEP * DAMPING);
        gravity_displacement += gravity_velocity * STEP;

        let required = [
            relative_position[0] + target_velocity[0] * time
                - inherited_velocity[0] * displacement_factor,
            relative_position[1] + target_velocity[1] * time
                - inherited_velocity[1] * displacement_factor
                - gravity_displacement,
        ];
        let distance = MUZZLE_OFFSET + speed * displacement_factor;
        let error = required[0].hypot(required[1]) - distance;
        if error > 0.0 {
            previous_error = error;
            continue;
        }

        // Interpolate inside the final physics step, rather than rounding flight time up.
        let fraction = previous_error / (previous_error - error);
        let flight = (step - 1) as f32 * STEP + fraction * STEP;
        let factor = previous_displacement_factor
            + fraction * (displacement_factor - previous_displacement_factor);
        let gravity = previous_gravity_displacement
            + fraction * (gravity_displacement - previous_gravity_displacement);
        let required = [
            relative_position[0] + target_velocity[0] * flight - inherited_velocity[0] * factor,
            relative_position[1] + target_velocity[1] * flight
                - inherited_velocity[1] * factor
                - gravity,
        ];
        let length = required[0].hypot(required[1]);
        if !length.is_finite() || length <= 0.0 {
            return None;
        }
        return Some(Solution {
            direction: [required[0] / length, required[1] / length],
            distance: MUZZLE_OFFSET + speed * factor,
            flight,
        });
    }
    None
}
