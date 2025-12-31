//! Example: Defining custom job types for domain-specific categorization
//!
//! This example shows how to create your own job types for domain-specific
//! routing with TypedThreadPool.
//!
//! Run with: `cargo run --example custom_job_type`

use rust_thread_system::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Custom job types for a game engine
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GameJobType {
    Physics,
    Rendering,
    Audio,
    Network,
    AI,
}

impl JobType for GameJobType {
    fn all_variants() -> &'static [Self] {
        &[
            Self::Physics,
            Self::Rendering,
            Self::Audio,
            Self::Network,
            Self::AI,
        ]
    }

    fn default_type() -> Self {
        Self::Physics
    }

    fn name(&self) -> String {
        match self {
            Self::Physics => "Physics".to_string(),
            Self::Rendering => "Rendering".to_string(),
            Self::Audio => "Audio".to_string(),
            Self::Network => "Network".to_string(),
            Self::AI => "AI".to_string(),
        }
    }
}

fn main() -> Result<()> {
    println!("=== Custom Job Type Example (Game Engine) ===\n");

    // Create a pool optimized for game workloads
    let config = TypedPoolConfig::<GameJobType>::new()
        .workers_for(GameJobType::Physics, 2) // Physics needs consistency
        .workers_for(GameJobType::Rendering, 4) // GPU-bound, needs parallelism
        .workers_for(GameJobType::Audio, 1) // Single audio thread
        .workers_for(GameJobType::Network, 4) // IO-bound, high concurrency
        .workers_for(GameJobType::AI, 2) // CPU-bound AI calculations
        .type_priority(vec![
            GameJobType::Rendering, // Frame rendering is critical
            GameJobType::Physics,   // Physics affects rendering
            GameJobType::Audio,     // Audio glitches are noticeable
            GameJobType::Network,   // Network can be slightly delayed
            GameJobType::AI,        // AI can be deferred
        ])
        .with_thread_name_prefix("game-worker");

    println!("Game Engine Thread Pool Configuration:");
    println!(
        "  Physics workers: {}",
        config.get_workers_for(GameJobType::Physics)
    );
    println!(
        "  Rendering workers: {}",
        config.get_workers_for(GameJobType::Rendering)
    );
    println!(
        "  Audio workers: {}",
        config.get_workers_for(GameJobType::Audio)
    );
    println!(
        "  Network workers: {}",
        config.get_workers_for(GameJobType::Network)
    );
    println!("  AI workers: {}", config.get_workers_for(GameJobType::AI));
    println!("  Total workers: {}", config.total_workers());
    println!();

    let pool = TypedThreadPool::new(config)?;
    pool.start()?;

    // Simulate a game frame
    println!("Simulating game frame...\n");

    let frame_counter = Arc::new(AtomicUsize::new(0));

    // Submit physics jobs
    for i in 0..5 {
        let counter = Arc::clone(&frame_counter);
        pool.execute_typed(GameJobType::Physics, move || {
            // Simulate physics calculation
            let _velocity: f64 = (i as f64) * 9.8;
            counter.fetch_add(1, Ordering::Relaxed);
            println!("[Physics] Calculated object {} physics", i);
            Ok(())
        })?;
    }

    // Submit rendering jobs
    for i in 0..10 {
        let counter = Arc::clone(&frame_counter);
        pool.execute_typed(GameJobType::Rendering, move || {
            // Simulate rendering
            thread::sleep(Duration::from_micros(100));
            counter.fetch_add(1, Ordering::Relaxed);
            println!("[Rendering] Rendered mesh {}", i);
            Ok(())
        })?;
    }

    // Submit audio jobs
    for i in 0..3 {
        let counter = Arc::clone(&frame_counter);
        pool.execute_typed(GameJobType::Audio, move || {
            counter.fetch_add(1, Ordering::Relaxed);
            println!("[Audio] Mixed audio source {}", i);
            Ok(())
        })?;
    }

    // Submit network jobs
    for i in 0..4 {
        let counter = Arc::clone(&frame_counter);
        pool.execute_typed(GameJobType::Network, move || {
            // Simulate network IO
            thread::sleep(Duration::from_millis(5));
            counter.fetch_add(1, Ordering::Relaxed);
            println!("[Network] Synced player {} position", i);
            Ok(())
        })?;
    }

    // Submit AI jobs
    for i in 0..3 {
        let counter = Arc::clone(&frame_counter);
        pool.execute_typed(GameJobType::AI, move || {
            // Simulate AI calculation
            let _decision = i % 2;
            counter.fetch_add(1, Ordering::Relaxed);
            println!("[AI] Calculated AI decision for NPC {}", i);
            Ok(())
        })?;
    }

    // Wait for frame to complete
    thread::sleep(Duration::from_millis(500));

    println!("\n=== Frame Statistics ===\n");
    println!(
        "Total jobs in frame: {}",
        frame_counter.load(Ordering::Relaxed)
    );

    // Print per-type statistics
    for job_type in GameJobType::all_variants() {
        if let Some(stats) = pool.type_stats(*job_type) {
            println!("{:?}:", job_type);
            println!(
                "  Completed: {}/{}",
                stats.jobs_completed, stats.jobs_submitted
            );
            println!("  Avg latency: {:?}", stats.avg_latency);
        }
    }

    // Shutdown
    println!("\nShutting down...");
    pool.shutdown()?;
    println!("Done!");

    Ok(())
}
