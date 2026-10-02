//! Runs the simulation on its own thread so the window stays smooth at any
//! sim speed. The renderer only ever reads the latest published [`Snapshot`].

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use bevy::prelude::Resource;
use sim_core::{Command, Simulation, World};

/// How fast the simulation runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Speed {
    Paused,
    TicksPerSecond(f64),
    Max,
}

pub const SPEEDS: [(&str, Speed); 6] = [
    ("Pause", Speed::Paused),
    ("1 month/s", Speed::TicksPerSecond(1.0)),
    ("1 year/s", Speed::TicksPerSecond(12.0)),
    ("10 years/s", Speed::TicksPerSecond(120.0)),
    ("100 years/s", Speed::TicksPerSecond(1200.0)),
    ("Max", Speed::Max),
];

enum Control {
    SetSpeed(Speed),
    Submit(Command),
}

/// What the view sees of the simulation.
#[derive(Clone)]
pub struct Snapshot {
    pub tick: u64,
    /// Republished only when the world's terrain changes, since cloning it
    /// every tick would be wasteful.
    pub world: Arc<World>,
    pub ticks_per_second: f64,
}

#[derive(Resource)]
pub struct SimThread {
    control: Sender<Control>,
    snapshot: Arc<Mutex<Snapshot>>,
    speed: Speed,
}

impl SimThread {
    pub fn spawn(mut sim: Simulation) -> Self {
        let snapshot = Arc::new(Mutex::new(Snapshot {
            tick: sim.world().tick,
            world: Arc::new(sim.world().clone()),
            ticks_per_second: 0.0,
        }));
        let (control, rx) = mpsc::channel();
        let shared = snapshot.clone();
        thread::Builder::new()
            .name("simulation".into())
            .spawn(move || run(&mut sim, &rx, &shared))
            .expect("failed to start simulation thread");
        SimThread { control, snapshot, speed: Speed::Paused }
    }

    pub fn speed(&self) -> Speed {
        self.speed
    }

    pub fn set_speed(&mut self, speed: Speed) {
        self.speed = speed;
        let _ = self.control.send(Control::SetSpeed(speed));
    }

    pub fn submit(&self, command: Command) {
        let _ = self.control.send(Control::Submit(command));
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().expect("simulation thread panicked").clone()
    }
}

fn run(sim: &mut Simulation, rx: &Receiver<Control>, shared: &Mutex<Snapshot>) {
    let mut speed = Speed::Paused;
    let mut published_revision = sim.world().terrain_revision;
    let mut next_tick = Instant::now();
    // Measured rate, over roughly one-second windows.
    let (mut window_start, mut window_ticks) = (Instant::now(), 0u64);

    loop {
        // Wait for control messages or the next scheduled tick.
        let wait = match speed {
            Speed::Paused => Duration::from_millis(100),
            Speed::Max => Duration::ZERO,
            Speed::TicksPerSecond(_) => next_tick.saturating_duration_since(Instant::now()),
        };
        match rx.recv_timeout(wait) {
            Ok(Control::SetSpeed(s)) => {
                speed = s;
                next_tick = Instant::now();
                continue;
            }
            Ok(Control::Submit(cmd)) => {
                sim.submit(cmd);
                continue;
            }
            Err(RecvTimeoutError::Timeout) => {}
            // The app has closed.
            Err(RecvTimeoutError::Disconnected) => return,
        }

        let batch = match speed {
            Speed::Paused => 0,
            Speed::Max => 64,
            Speed::TicksPerSecond(tps) => {
                next_tick += Duration::from_secs_f64(1.0 / tps);
                // Don't try to catch up after a stall; just carry on.
                if next_tick < Instant::now() {
                    next_tick = Instant::now();
                }
                1
            }
        };
        sim.run(batch);
        window_ticks += batch;

        let mut snap = shared.lock().expect("view thread panicked");
        snap.tick = sim.world().tick;
        if sim.world().terrain_revision != published_revision {
            published_revision = sim.world().terrain_revision;
            snap.world = Arc::new(sim.world().clone());
        }
        let elapsed = window_start.elapsed().as_secs_f64();
        if elapsed >= 1.0 || speed == Speed::Paused {
            snap.ticks_per_second = window_ticks as f64 / elapsed.max(1e-9);
            (window_start, window_ticks) = (Instant::now(), 0);
        }
    }
}
