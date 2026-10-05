//! Runs the simulation on its own thread so the window stays smooth at any
//! sim speed. The view only ever reads the latest published [`Snapshot`].

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use bevy::prelude::Resource;
use sim_core::{Command, HexId, Simulation, World};

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
    Save { path: PathBuf, settings: String, reply: Sender<Result<PathBuf, String>> },
}

/// What the view sees of the simulation.
#[derive(Clone)]
pub struct Snapshot {
    pub tick: u64,
    /// Republished only when the world changes in a way the view draws,
    /// since cloning it every tick would be wasteful.
    pub world: Arc<World>,
    pub ticks_per_second: f64,
    /// How many edits can be undone and redone.
    pub undo_redo: (usize, usize),
}

struct Shared {
    snapshot: Mutex<Snapshot>,
    /// Hexes changed since the view last asked, so it can redraw just those.
    changed: Mutex<Vec<HexId>>,
}

/// Handle to the simulation thread. Dropping it stops the thread.
#[derive(Resource)]
pub struct SimThread {
    control: Sender<Control>,
    shared: Arc<Shared>,
    speed: Speed,
}

impl SimThread {
    pub fn spawn(mut sim: Simulation) -> Self {
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Snapshot {
                tick: sim.world().tick,
                world: Arc::new(sim.world().clone()),
                ticks_per_second: 0.0,
                undo_redo: (0, 0),
            }),
            changed: Mutex::new(Vec::new()),
        });
        let (control, rx) = mpsc::channel();
        let thread_shared = shared.clone();
        thread::Builder::new()
            .name("simulation".into())
            .spawn(move || run(&mut sim, &rx, &thread_shared))
            .expect("failed to start simulation thread");
        SimThread { control, shared, speed: Speed::Paused }
    }

    pub fn speed(&self) -> Speed {
        self.speed
    }

    pub fn set_speed(&mut self, speed: Speed) {
        self.speed = speed;
        let _ = self.control.send(Control::SetSpeed(speed));
    }

    /// Sends a command. It's applied as soon as the simulation is between
    /// ticks, which is immediately when paused.
    pub fn submit(&self, command: Command) {
        let _ = self.control.send(Control::Submit(command));
    }

    /// Saves the world as it stands between ticks. The file is written in
    /// the background; the result arrives on the returned channel.
    pub fn save(&self, path: PathBuf, settings: String) -> Receiver<Result<PathBuf, String>> {
        let (reply, result) = mpsc::channel();
        let _ = self.control.send(Control::Save { path, settings, reply });
        result
    }

    pub fn snapshot(&self) -> Snapshot {
        self.shared.snapshot.lock().expect("simulation thread panicked").clone()
    }

    pub fn take_changed_hexes(&self) -> Vec<HexId> {
        std::mem::take(&mut *self.shared.changed.lock().expect("simulation thread panicked"))
    }
}

fn run(sim: &mut Simulation, rx: &Receiver<Control>, shared: &Shared) {
    let mut speed = Speed::Paused;
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
        let mut batch = 0;
        match rx.recv_timeout(wait) {
            Ok(Control::SetSpeed(s)) => {
                speed = s;
                next_tick = Instant::now();
            }
            Ok(Control::Submit(cmd)) => {
                sim.submit(cmd);
                // Drain any queued commands too (e.g. a fast brush stroke)
                // so they're published together.
                while let Ok(msg) = rx.try_recv() {
                    match msg {
                        Control::Submit(cmd) => sim.submit(cmd),
                        Control::SetSpeed(s) => speed = s,
                        Control::Save { path, settings, reply } => {
                            sim.apply_pending();
                            save(sim, path, settings, reply);
                        }
                    }
                }
                sim.apply_pending();
            }
            Ok(Control::Save { path, settings, reply }) => {
                sim.apply_pending();
                save(sim, path, settings, reply);
            }
            Err(RecvTimeoutError::Timeout) => {
                batch = match speed {
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
            }
            // The app has closed or started a new world.
            Err(RecvTimeoutError::Disconnected) => return,
        }

        sim.run(batch);
        window_ticks += batch;
        publish(sim, shared);

        let elapsed = window_start.elapsed().as_secs_f64();
        if elapsed >= 1.0 || speed == Speed::Paused {
            shared.snapshot.lock().expect("view thread panicked").ticks_per_second =
                window_ticks as f64 / elapsed.max(1e-9);
            (window_start, window_ticks) = (Instant::now(), 0);
        }
    }
}

/// Copies the state now and writes it on another thread, so a big world
/// doesn't stall the simulation while it's compressed.
fn save(sim: &Simulation, path: PathBuf, settings: String, reply: Sender<Result<PathBuf, String>>) {
    let data = sim.save_data(settings);
    let registry = sim.registry().clone();
    thread::spawn(move || {
        let result = sim_core::save::save(&path, &data, &registry).map(|()| path).map_err(|e| e.to_string());
        let _ = reply.send(result);
    });
}

fn publish(sim: &mut Simulation, shared: &Shared) {
    let changed = sim.take_changed_hexes();
    {
        let mut snap = shared.snapshot.lock().expect("view thread panicked");
        snap.tick = sim.world().tick;
        snap.undo_redo = sim.undo_redo_depth();
        if !changed.is_empty() {
            snap.world = Arc::new(sim.world().clone());
        }
    }
    // After the world is published, so the view never redraws stale terrain.
    if !changed.is_empty() {
        shared.changed.lock().expect("view thread panicked").extend(changed);
    }
}
