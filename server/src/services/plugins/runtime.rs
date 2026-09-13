//! Instantiate a plugin module, call an export, bound it with a deadline,
//! and turn whatever goes wrong into a value instead of a panic.
//!
//! There is no host API surface here (see [RCS-256]'s follow-up tickets for
//! `invoice_get` and friends) and no imports at all: today's plugin modules
//! are self-contained, so instantiation needs no [`wasmtime::Linker`]. The
//! calling convention is host-defined, not part of `payserver-plugin-api`
//! yet: a plugin exports `memory`, an `alloc(len: i32) -> i32` bump
//! allocator the host uses to place its (JSON) argument, and one function
//! per call name with signature `(ptr: i32, len: i32) -> i64`, the result
//! packed as `(ptr << 32) | len` into the return value.
//!
//! [RCS-256]: https://linear.app/randomcash/issue/RCS-256

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use wasmtime::{Config, Engine, Instance, Memory, Module, Store, TypedFunc};

/// Ticks the shared [`Engine`]'s epoch on a fixed cadence so a [`Store`]'s
/// deadline (set in ticks, not wall time) actually elapses.
///
/// One ticker per engine, not per call: the epoch is engine-wide, and a
/// plugin instance only ever runs one call at a time (see
/// [`super::host::PluginEntry`]'s call mutex), so there is nothing for a
/// second ticker to do.
struct EpochTicker {
    running: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl EpochTicker {
    fn spawn(engine: Engine, interval: Duration) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let flag = running.clone();
        let handle = std::thread::spawn(move || {
            while flag.load(Ordering::Relaxed) {
                std::thread::sleep(interval);
                engine.increment_epoch();
            }
        });
        Self {
            running,
            handle: Some(handle),
        }
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// The compiled-module factory and epoch clock shared by every plugin the
/// host loads. Created once and kept for the process lifetime.
pub struct PluginEngine {
    engine: Engine,
    tick: Duration,
    _ticker: EpochTicker,
}

impl PluginEngine {
    /// A tick short enough that tests can use millisecond-scale deadlines
    /// without either flaking (too coarse) or busy-looping (too fine).
    const DEFAULT_TICK: Duration = Duration::from_millis(5);

    pub fn new() -> Self {
        Self::with_tick(Self::DEFAULT_TICK)
    }

    pub fn with_tick(tick: Duration) -> Self {
        let mut config = Config::new();
        config.epoch_interruption(true);
        // `Config::new` here is hardcoded and known-valid; `Engine::new` only
        // fails for a config wasmtime can't support on this host, which is
        // not a runtime condition we need to recover from.
        #[allow(clippy::expect_used)]
        let engine = Engine::new(&config).expect("hardcoded wasmtime Config is always valid");
        let ticker = EpochTicker::spawn(engine.clone(), tick);
        Self {
            engine,
            tick,
            _ticker: ticker,
        }
    }

    /// Compiles `wasm` into a [`Module`] ready to instantiate.
    pub fn compile(&self, wasm: &[u8]) -> Result<Module, PluginWasmError> {
        Module::new(&self.engine, wasm).map_err(|e| PluginWasmError::Compile(e.to_string()))
    }

    /// Instantiates `module`, once, keeping the resulting store and export
    /// handles for reuse across calls.
    pub fn instantiate(&self, module: &Module) -> Result<PluginInstance, PluginWasmError> {
        PluginInstance::new(&self.engine, module)
    }

    /// The number of epoch ticks that will elapse in at least `deadline`,
    /// rounded up and never zero — a zero deadline would mean "already
    /// expired" rather than "as soon as possible".
    pub fn ticks_for(&self, deadline: Duration) -> u64 {
        let tick_nanos = self.tick.as_nanos().max(1);
        let deadline_nanos = deadline.as_nanos();
        deadline_nanos.div_ceil(tick_nanos).max(1) as u64
    }
}

impl Default for PluginEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// A plugin's own memory allocation call, kept type-checked once it's
/// resolved rather than an export name looked up on every call.
type Alloc = TypedFunc<i32, i32>;

/// A single loaded, instantiated plugin: a [`Store`] plus the two export
/// handles every call needs. Kept alive and reused across calls rather than
/// re-instantiated each time.
pub struct PluginInstance {
    store: Store<()>,
    instance: Instance,
    memory: Memory,
    alloc: Alloc,
}

impl PluginInstance {
    fn new(engine: &Engine, module: &Module) -> Result<Self, PluginWasmError> {
        let mut store = Store::new(engine, ());
        let instance = Instance::new(&mut store, module, &[])
            .map_err(|e| PluginWasmError::Instantiate(e.to_string()))?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| PluginWasmError::MissingExport("memory".to_string()))?;
        let alloc: Alloc = instance
            .get_typed_func(&mut store, "alloc")
            .map_err(|_| PluginWasmError::MissingExport("alloc".to_string()))?;
        Ok(Self {
            store,
            instance,
            memory,
            alloc,
        })
    }

    /// Calls the export named `export` with `arg_json` (already-serialised
    /// JSON bytes) and returns the raw JSON bytes it answered with.
    ///
    /// `deadline_ticks` bounds the call: if the engine's epoch advances past
    /// it before the call returns, wasmtime traps the call in place and this
    /// returns [`PluginCallError::DeadlineExceeded`] rather than blocking
    /// forever on a plugin that never returns.
    pub fn call_raw(
        &mut self,
        export: &str,
        arg_json: &[u8],
        deadline_ticks: u64,
    ) -> Result<Vec<u8>, PluginCallError> {
        self.store.set_epoch_deadline(deadline_ticks);

        let len = i32::try_from(arg_json.len())
            .map_err(|_| PluginCallError::Other("argument too large for a wasm32 plugin".into()))?;
        let ptr = self.alloc.call(&mut self.store, len).map_err(classify)?;
        self.memory
            .write(&mut self.store, ptr as usize, arg_json)
            .map_err(|e| PluginCallError::Other(e.to_string()))?;

        let call: TypedFunc<(i32, i32), i64> = self
            .instance
            .get_typed_func(&mut self.store, export)
            .map_err(|_| PluginCallError::MissingExport(export.to_string()))?;
        let packed = call.call(&mut self.store, (ptr, len)).map_err(classify)?;

        let (out_ptr, out_len) = unpack(packed);
        if out_len == 0 {
            return Ok(Vec::new());
        }
        let mut buf = vec![0u8; out_len as usize];
        self.memory
            .read(&self.store, out_ptr as usize, &mut buf)
            .map_err(|e| PluginCallError::Other(e.to_string()))?;
        Ok(buf)
    }

    /// [`call_raw`](Self::call_raw), serialising `req` to JSON and
    /// deserialising the answer as `Resp`. A plugin that answers with bytes
    /// that are not valid `Resp` JSON — garbage, a different shape, garbled
    /// output from a confused plugin — fails here cleanly, as a
    /// [`PluginCallError::Deserialize`], rather than panicking the host.
    pub fn call<Req: Serialize, Resp: DeserializeOwned>(
        &mut self,
        export: &str,
        req: &Req,
        deadline_ticks: u64,
    ) -> Result<Resp, PluginCallError> {
        let arg = serde_json::to_vec(req).map_err(|e| {
            PluginCallError::Other(format!("could not serialise call argument: {e}"))
        })?;
        let bytes = self.call_raw(export, &arg, deadline_ticks)?;
        serde_json::from_slice(&bytes).map_err(|e| PluginCallError::Deserialize(e.to_string()))
    }
}

/// The host never packs — only the test fixtures below do, standing in for
/// a real plugin's own encoding of its answer.
#[cfg(test)]
fn pack(ptr: u32, len: u32) -> i64 {
    (((ptr as u64) << 32) | (len as u64)) as i64
}

fn unpack(packed: i64) -> (u32, u32) {
    let packed = packed as u64;
    ((packed >> 32) as u32, packed as u32)
}

/// Turns a raw wasmtime call error into a [`PluginCallError`], picking out
/// epoch-deadline traps specifically so the caller can tell "the plugin ran
/// too long" apart from "the plugin crashed".
fn classify(err: wasmtime::Error) -> PluginCallError {
    match err.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::Interrupt) => PluginCallError::DeadlineExceeded,
        Some(trap) => PluginCallError::Trap(trap.to_string()),
        None => PluginCallError::Other(err.to_string()),
    }
}

/// Why a plugin module could not be loaded and instantiated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluginWasmError {
    #[error("failed to compile plugin module: {0}")]
    Compile(String),
    #[error("failed to instantiate plugin module: {0}")]
    Instantiate(String),
    #[error("plugin module is missing required export {0:?}")]
    MissingExport(String),
}

/// Why a single call into an already-instantiated plugin failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluginCallError {
    #[error("plugin export {0:?} not found")]
    MissingExport(String),
    /// The call's deadline elapsed before the plugin returned.
    #[error("plugin exceeded its deadline")]
    DeadlineExceeded,
    /// The plugin panicked (in wasm terms: trapped) for a reason other than
    /// the deadline — `unreachable`, an out-of-bounds access, and so on.
    #[error("plugin trapped: {0}")]
    Trap(String),
    /// The call completed, but its answer did not deserialise as expected.
    #[error("plugin returned an answer the host could not parse: {0}")]
    Deserialize(String),
    #[error("plugin call failed: {0}")]
    Other(String),
}

/// Test-only WAT fixtures for the ticket's five failure modes, shared with
/// [`super::host`]'s tests — `pub(crate)` rather than duplicated, since both
/// modules need the same panicking/looping/garbage-answer plugins.
#[cfg(test)]
pub(crate) mod fixtures {
    #![allow(clippy::unwrap_used)]

    use super::{PluginEngine, PluginInstance};

    /// A plugin exporting a bump allocator and one `call` per fixture
    /// behaviour, hand-written in WAT so tests need no wasm32 toolchain.
    fn wat_module(call_body: &str) -> Vec<u8> {
        let text = format!(
            r#"
            (module
                (memory (export "memory") 1)
                (global $next (mut i32) (i32.const 1024))

                (func (export "alloc") (param $len i32) (result i32)
                    (local $ptr i32)
                    (local.set $ptr (global.get $next))
                    (global.set $next (i32.add (global.get $next) (local.get $len)))
                    (local.get $ptr))

                (func (export "call") (param $ptr i32) (param $len i32) (result i64)
                    {call_body})
            )
            "#
        );
        wat::parse_str(text).unwrap()
    }

    /// Echoes the argument straight back: `call` returns the same
    /// `(ptr, len)` it was given, packed.
    pub(crate) fn echo_module() -> Vec<u8> {
        wat_module(
            r#"
            (i64.or
                (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
                (i64.extend_i32_u (local.get $len)))
            "#,
        )
    }

    pub(crate) fn panicking_module() -> Vec<u8> {
        wat_module("unreachable")
    }

    pub(crate) fn infinite_loop_module() -> Vec<u8> {
        // The trailing `unreachable` is dead code — the loop only exits via
        // an epoch trap — but wasm's validator type-checks a `loop` by its
        // declared (empty) block type rather than proving it never falls
        // through, so the function body still needs *something* of type i64
        // after it to satisfy the declared return type.
        wat_module(r#"(loop $forever (br $forever)) unreachable"#)
    }

    /// Ignores its argument and returns a fixed pointer/length pointing at a
    /// data segment that is not valid JSON. Needs its own module body
    /// (rather than `wat_module`) to declare that data segment.
    pub(crate) fn garbage_module() -> Vec<u8> {
        let text = r#"
            (module
                (memory (export "memory") 1)
                (data (i32.const 0) "not json {")
                (global $next (mut i32) (i32.const 1024))

                (func (export "alloc") (param $len i32) (result i32)
                    (local $ptr i32)
                    (local.set $ptr (global.get $next))
                    (global.set $next (i32.add (global.get $next) (local.get $len)))
                    (local.get $ptr))

                (func (export "call") (param $ptr i32) (param $len i32) (result i64)
                    (i64.or
                        (i64.shl (i64.extend_i32_u (i32.const 0)) (i64.const 32))
                        (i64.extend_i32_u (i32.const 10))))
            )
        "#;
        wat::parse_str(text).unwrap()
    }

    pub(crate) fn instance_for(engine: &PluginEngine, wasm: &[u8]) -> PluginInstance {
        let module = engine.compile(wasm).unwrap();
        engine.instantiate(&module).unwrap()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::time::{Duration, Instant};

    use serde::{Deserialize, Serialize};

    use super::fixtures::*;
    use super::*;

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct Ping {
        n: u32,
    }

    #[test]
    fn calls_an_export_and_deserialises_the_answer() {
        let engine = PluginEngine::with_tick(Duration::from_millis(5));
        let mut plugin = instance_for(&engine, &echo_module());

        let resp: Ping = plugin
            .call(
                "call",
                &Ping { n: 7 },
                engine.ticks_for(Duration::from_secs(1)),
            )
            .unwrap();

        assert_eq!(resp, Ping { n: 7 });
    }

    /// Ticket test 1: a panicking plugin traps, and the host gets an `Err`
    /// back rather than a Rust panic unwinding into the caller.
    #[test]
    fn catches_a_trap_without_panicking() {
        let engine = PluginEngine::with_tick(Duration::from_millis(5));
        let mut plugin = instance_for(&engine, &panicking_module());

        let err = plugin
            .call::<_, Ping>(
                "call",
                &Ping { n: 1 },
                engine.ticks_for(Duration::from_secs(1)),
            )
            .unwrap_err();

        assert!(matches!(err, PluginCallError::Trap(_)), "got {err:?}");
    }

    /// Ticket test 2: a plugin that never returns is interrupted once the
    /// deadline elapses, and the call returns in bounded time rather than
    /// hanging forever.
    #[test]
    fn deadline_fires_on_a_plugin_that_never_returns() {
        let engine = PluginEngine::with_tick(Duration::from_millis(5));
        let mut plugin = instance_for(&engine, &infinite_loop_module());

        let started = Instant::now();
        let err = plugin
            .call::<_, Ping>(
                "call",
                &Ping { n: 1 },
                engine.ticks_for(Duration::from_millis(20)),
            )
            .unwrap_err();
        let elapsed = started.elapsed();

        assert_eq!(err, PluginCallError::DeadlineExceeded);
        assert!(
            elapsed < Duration::from_secs(5),
            "took {elapsed:?} to interrupt"
        );
    }

    /// Ticket test 3: a plugin returning bytes that are not valid JSON for
    /// the expected type fails deserialisation cleanly.
    #[test]
    fn garbage_bytes_fail_deserialisation_cleanly() {
        let engine = PluginEngine::with_tick(Duration::from_millis(5));
        let mut plugin = instance_for(&engine, &garbage_module());

        let err = plugin
            .call::<_, Ping>(
                "call",
                &Ping { n: 1 },
                engine.ticks_for(Duration::from_secs(1)),
            )
            .unwrap_err();

        assert!(
            matches!(err, PluginCallError::Deserialize(_)),
            "got {err:?}"
        );
    }

    /// A store that has trapped is still usable for the next call — the
    /// premise behind "instantiate once and keep", not re-instantiate on
    /// every failure.
    #[test]
    fn a_trapped_instance_can_still_be_called_again() {
        let engine = PluginEngine::with_tick(Duration::from_millis(5));
        let mut plugin = instance_for(&engine, &panicking_module());

        assert!(
            plugin
                .call::<_, Ping>(
                    "call",
                    &Ping { n: 1 },
                    engine.ticks_for(Duration::from_secs(1))
                )
                .is_err()
        );
        assert!(
            plugin
                .call::<_, Ping>(
                    "call",
                    &Ping { n: 1 },
                    engine.ticks_for(Duration::from_secs(1))
                )
                .is_err(),
            "the same trap should reproduce, not corrupt the store into something else"
        );
    }

    #[test]
    fn pack_unpack_roundtrips() {
        assert_eq!(unpack(pack(1234, 5678)), (1234, 5678));
        assert_eq!(unpack(pack(0, 0)), (0, 0));
        assert_eq!(unpack(pack(u32::MAX, u32::MAX)), (u32::MAX, u32::MAX));
    }

    #[test]
    fn ticks_for_is_never_zero() {
        let engine = PluginEngine::with_tick(Duration::from_millis(10));
        assert_eq!(engine.ticks_for(Duration::from_millis(0)), 1);
        assert_eq!(engine.ticks_for(Duration::from_millis(1)), 1);
        assert_eq!(engine.ticks_for(Duration::from_millis(10)), 1);
        assert_eq!(engine.ticks_for(Duration::from_millis(11)), 2);
    }
}
