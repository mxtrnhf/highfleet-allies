//! Synthetic broadphase benchmark; not game frame times or native equivalence.
//! rustc --edition 2021 -O examples/spatial_query.rs -o spatial-query-bench

#[path = "../src/spatial.rs"]
mod spatial;

use spatial::{Bounds, Ray, SpatialIndex};
use std::{hint::black_box, time::Instant};

fn main() -> Result<(), &'static str> {
    let rays: Vec<_> = (0..384)
        .map(|index| {
            let angle = index as f32 * std::f32::consts::TAU / 384.0;
            Ray {
                start: [0.0, 0.0],
                end: [angle.cos() * 1000.0, angle.sin() * 1000.0],
                max_fraction: 1.0,
            }
        })
        .collect();
    println!("fixtures,rays,round,linear_ms,index_build_and_query_ms,candidates,full_scan_entries");
    for count in [512, 4096, 16384] {
        let bounds: Vec<_> = (0..count)
            .map(|index| {
                let x = (index % 128) as f32 * 8.0 - 512.0;
                let y = (index / 128) as f32 * 8.0 - 512.0;
                Bounds {
                    lower: [x - 1.0, y - 1.0],
                    upper: [x + 1.0, y + 1.0],
                }
            })
            .collect();
        let mut index = SpatialIndex::default();
        let mut candidates = Vec::with_capacity(count);
        index.build(&bounds)?;
        // Validate benchmark outputs before timing: every broadphase survivor
        // and its ordinal must agree, not just the nearest candidate or count.
        for &ray in &rays {
            if !index.candidates(ray, &mut candidates) {
                return Err("candidate query failed");
            }
            let linear: Vec<_> = bounds
                .iter()
                .enumerate()
                .filter_map(|(ordinal, &bounds)| bounds.intersects(ray).then_some(ordinal))
                .collect();
            if linear != candidates {
                return Err("BVH/linear candidate mismatch");
            }
        }
        for round in 0..7 {
            let start = Instant::now();
            let mut linear_count = 0;
            for &ray in black_box(&rays) {
                for &bounds in black_box(&bounds) {
                    linear_count += usize::from(bounds.intersects(ray));
                }
            }
            let linear = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            index.build(black_box(&bounds))?;
            let mut indexed_count = 0;
            for &ray in &rays {
                if !index.candidates(black_box(ray), &mut candidates) {
                    return Err("candidate query failed");
                }
                indexed_count += black_box(candidates.len());
            }
            let indexed = start.elapsed().as_secs_f64() * 1000.0;
            if indexed_count != linear_count {
                return Err("candidate count changed during measurement");
            }
            println!(
                "{count},{},{round},{linear:.6},{indexed:.6},{indexed_count},{}",
                rays.len(),
                count * rays.len()
            );
        }
    }
    eprintln!("Synthetic full-candidate/order comparison passed. No game was launched.");
    Ok(())
}
