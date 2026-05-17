use std::path::PathBuf;

use anyhow::{Context, Result};
use wasmtime::component::{Component, HasSelf, Linker, Resource, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

// ── WIT bindings ─────────────────────────────────────────────────────────────

/// Resource type stored per corpus-handle in the ResourceTable.
/// M1: stub (no real parquet). Later: holds a parquet reader / slab index.
pub struct CorpusHandle;

/// Resource type stored per row-stream in the ResourceTable.
pub struct RowStream {
    rows: Vec<superstition::detector::corpus::Row>,
    pos: usize,
}

wasmtime::component::bindgen!({
    path: "../../wit",
    world: "detector-world",
    with: {
        "superstition:detector/corpus.corpus-handle": CorpusHandle,
        "superstition:detector/corpus.row-stream": RowStream,
    },
});

use superstition::detector::corpus::{CorpusError, Host, HostCorpusHandle, HostRowStream, Row};

// ── host state ────────────────────────────────────────────────────────────────

struct State {
    table: ResourceTable,
    wasi: WasiCtx,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

// ── corpus interface implementation (stub for M1) ─────────────────────────────

impl HostCorpusHandle for State {
    fn drop(&mut self, handle: Resource<CorpusHandle>) -> wasmtime::Result<()> {
        self.table.delete(handle)?;
        Ok(())
    }
}

impl HostRowStream for State {
    fn next(&mut self, self_: Resource<RowStream>) -> Option<Row> {
        // next() is infallible in WIT; on table error we just stop the stream.
        let stream = self.table.get_mut(&self_).ok()?;
        if stream.pos < stream.rows.len() {
            let row = stream.rows[stream.pos].clone();
            stream.pos += 1;
            Some(row)
        } else {
            None
        }
    }

    fn drop(&mut self, stream: Resource<RowStream>) -> wasmtime::Result<()> {
        self.table.delete(stream)?;
        Ok(())
    }
}

impl Host for State {
    fn iterator(
        &mut self,
        _handle: Resource<CorpusHandle>,
        table: String,
    ) -> Result<Resource<RowStream>, CorpusError> {
        if table != "erc20_transfers" {
            return Err(CorpusError::NoSuchTable(table));
        }
        let rows = stub_erc20_rows();
        self.table
            .push(RowStream { rows, pos: 0 })
            .map_err(|e| CorpusError::Internal(e.to_string()))
    }
}

// ── stub corpus data (M1: one row per day of the week) ────────────────────────

/// Returns 7 rows: one per day of the week starting 2023-01-01 (Sunday).
///
/// (epoch day 19358; (19358 + 4) % 7 == 0 → index 0 = Sunday)
fn stub_erc20_rows() -> Vec<Row> {
    use superstition::detector::corpus::Value;
    let base_ts: u64 = 1_672_531_200; // 2023-01-01 00:00:00 UTC
    (0u64..7)
        .map(|day| Row {
            fields: vec![(
                "block_timestamp".to_string(),
                Value::U64Val(base_ts + day * 86_400),
            )],
        })
        .collect()
}

// ── main ─────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let wasm_path = PathBuf::from(
        std::env::args().nth(1).context("usage: host <path/to/detector.wasm>")?,
    );

    // ── engine: component model + epoch interruption ──
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.epoch_interruption(true);
    let engine = Engine::new(&config)?;

    // ── linker: WASI (minimal config) + corpus ──
    let mut linker: Linker<State> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    superstition::detector::corpus::add_to_linker::<State, HasSelf<State>>(
        &mut linker,
        |s| s,
    )?;

    // ── store: no preopens, no env, no stdio ──
    let wasi = WasiCtxBuilder::new().build();
    let mut store = Store::new(&engine, State { table: ResourceTable::new(), wasi });

    // Set epoch deadline (units = engine epoch ticks; we don't run a ticker in
    // M1 since the stub corpus is trivial, but the plumbing is wired).
    store.set_epoch_deadline(300);

    // ── load + instantiate the detector component ──
    let component = Component::from_file(&engine, &wasm_path)
        .map_err(|e| anyhow::anyhow!("loading {}: {e}", wasm_path.display()))?;
    let world = DetectorWorld::instantiate(&mut store, &component, &linker)?;

    // ── call describe() ──
    let meta = world.superstition_detector_detector().call_describe(&mut store)?;
    println!("description : {}", meta.description);
    println!("hypothesis  : {}", meta.hypothesis);
    println!("family      : {}", meta.family);
    println!("version     : {}", meta.version);
    println!();

    // ── push a corpus handle, call test() ──
    let handle = store.data_mut().table.push(CorpusHandle)?;
    let result = world.superstition_detector_detector().call_test(&mut store, handle)?;

    match result {
        Ok(r) => {
            println!("counts      : {:?}", r.counts);
            println!("sample_size : {}", r.sample_size);
            println!("test_type   : {:?}", r.test_type);
            println!("detail      : {}", r.detail);
        }
        Err(e) => eprintln!("detector error: {e:?}"),
    }

    Ok(())
}
