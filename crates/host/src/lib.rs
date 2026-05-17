use std::path::Path;

use anyhow::Result;
use wasmtime::component::{Component, HasSelf, Linker, Resource, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use superstition_corpus::Corpus;
use superstition_scorer::{score, DetectorOutput, TestType as ScorerTestType};

// ── WIT bindings ──────────────────────────────────────────────────────────────

pub struct CorpusHandle;

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
    corpus: Option<Corpus>,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl HostCorpusHandle for State {
    fn drop(&mut self, handle: Resource<CorpusHandle>) -> wasmtime::Result<()> {
        self.table.delete(handle)?;
        Ok(())
    }
}

impl HostRowStream for State {
    fn next(&mut self, self_: Resource<RowStream>) -> Option<Row> {
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
        let rows = if let Some(corpus) = &self.corpus {
            corpus
                .rows(&table)
                .map_err(|e| CorpusError::Internal(e.to_string()))?
                .into_iter()
                .map(corpus_row_to_wit)
                .collect()
        } else {
            if table != "erc20_transfers" {
                return Err(CorpusError::NoSuchTable(table));
            }
            stub_erc20_rows()
        };

        self.table
            .push(RowStream { rows, pos: 0 })
            .map_err(|e| CorpusError::Internal(e.to_string()))
    }
}

// ── value conversion ──────────────────────────────────────────────────────────

fn corpus_row_to_wit(r: superstition_corpus::Row) -> Row {
    use superstition::detector::corpus::Value as W;
    use superstition_corpus::Value as C;

    Row {
        fields: r
            .fields
            .into_iter()
            .map(|(k, v)| {
                let w = match v {
                    C::U64(n) => W::U64Val(n),
                    C::I64(n) => W::I64Val(n),
                    C::F64(f) => W::F64Val(f),
                    C::Str(s) => W::StringVal(s),
                    C::Bytes(b) => W::BytesVal(b),
                    C::Bool(b) => W::BoolVal(b),
                    C::Null => W::NullVal,
                };
                (k, w)
            })
            .collect(),
    }
}

fn stub_erc20_rows() -> Vec<Row> {
    use superstition::detector::corpus::Value;
    let base: u64 = 1_672_531_200;
    (0u64..7)
        .map(|d| Row {
            fields: vec![("block_timestamp".to_string(), Value::U64Val(base + d * 86_400))],
        })
        .collect()
}

fn wit_tt_to_scorer(tt: &exports::superstition::detector::detector::TestType) -> ScorerTestType {
    use exports::superstition::detector::detector::TestType as W;
    match tt {
        W::FisherExact => ScorerTestType::FisherExact,
        W::ChiSquared(df) => ScorerTestType::ChiSquared(*df),
        W::KolmogorovSmirnov => ScorerTestType::KolmogorovSmirnov,
        W::Bootstrap(p) => ScorerTestType::Bootstrap {
            statistic_name: p.statistic_name.clone(),
            permutations: p.permutations,
        },
    }
}

// ── public API ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RunResult {
    pub description: String,
    pub hypothesis: String,
    pub family: String,
    pub counts: Vec<u64>,
    pub sample_size: u64,
    pub detail: String,
    pub p_value: f64,
    pub effect_size: f64,
    pub passes_effect_floor: bool,
}

/// Reusable wasmtime executor.  Create once, call `run()` many times.
///
/// A background thread ticks the engine epoch at 1 Hz.  The per-run deadline
/// (default 300 ticks = 5 min) is enforced by wasmtime epoch interruption.
pub struct Executor {
    engine: Engine,
    linker: Linker<State>,
    /// Number of epoch ticks before a run is interrupted.
    epoch_deadline: u64,
}

impl Executor {
    /// Standard executor — 300-second (5-minute) hard timeout per run.
    pub fn new() -> Result<Self> {
        Self::with_deadline(300)
    }

    /// Executor with a custom epoch deadline (ticks = seconds with the 1 Hz ticker).
    pub fn with_deadline(epoch_deadline: u64) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config)?;

        // ── epoch ticker (1 Hz) ──────────────────────────────────────────
        // Without this thread, epoch_interruption does nothing.
        // The thread is intentionally leaked — it runs for the process lifetime.
        {
            let engine_tick = engine.clone();
            std::thread::Builder::new()
                .name("epoch-ticker".into())
                .spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    engine_tick.increment_epoch();
                })
                .expect("failed to spawn epoch ticker");
        }

        let mut linker: Linker<State> = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        superstition::detector::corpus::add_to_linker::<State, HasSelf<State>>(
            &mut linker,
            |s| s,
        )?;

        Ok(Self { engine, linker, epoch_deadline })
    }

    /// Run a detector wasm file against the given corpus (or stub if None).
    pub fn run(&self, wasm_path: &Path, corpus: Option<&Corpus>) -> Result<RunResult> {
        let wasi = WasiCtxBuilder::new().build();
        let mut store = Store::new(
            &self.engine,
            State { table: ResourceTable::new(), wasi, corpus: corpus.cloned() },
        );
        store.set_epoch_deadline(self.epoch_deadline);

        let component = Component::from_file(&self.engine, wasm_path)
            .map_err(|e| anyhow::anyhow!("loading {}: {e}", wasm_path.display()))?;
        let world = DetectorWorld::instantiate(&mut store, &component, &self.linker)?;

        let meta = world.superstition_detector_detector().call_describe(&mut store)?;
        let handle = store.data_mut().table.push(CorpusHandle)?;
        let result = world
            .superstition_detector_detector()
            .call_test(&mut store, handle)?
            .map_err(|e| anyhow::anyhow!("detector: {e:?}"))?;

        // ── output validation ────────────────────────────────────────────
        validate_output(&result)?;

        let output = DetectorOutput {
            counts: result.counts.clone(),
            sample_size: result.sample_size,
            test_type: wit_tt_to_scorer(&result.test_type),
            detail: result.detail.clone(),
        };
        let scored = score(&output).map_err(|e| anyhow::anyhow!("scorer: {e}"))?;

        Ok(RunResult {
            description: meta.description,
            hypothesis: meta.hypothesis,
            family: meta.family,
            counts: result.counts,
            sample_size: result.sample_size,
            detail: result.detail,
            p_value: scored.p_value,
            effect_size: scored.effect_size,
            passes_effect_floor: scored.passes_effect_floor,
        })
    }
}

// ── output validation ─────────────────────────────────────────────────────────

fn validate_output(r: &exports::superstition::detector::detector::TestResult) -> Result<()> {
    use exports::superstition::detector::detector::TestType as T;

    if r.sample_size == 0 {
        anyhow::bail!("detector returned sample_size = 0");
    }
    if r.counts.is_empty() {
        anyhow::bail!("detector returned empty counts");
    }

    match &r.test_type {
        T::FisherExact => {
            if r.counts.len() != 4 {
                anyhow::bail!(
                    "fisher_exact requires exactly 4 counts [a,b,c,d], got {}",
                    r.counts.len()
                );
            }
        }
        T::ChiSquared(df) => {
            let expected = *df as usize + 1;
            if r.counts.len() != expected {
                anyhow::bail!(
                    "chi_squared(df={df}) requires {expected} counts, got {}",
                    r.counts.len()
                );
            }
            if *df == 0 {
                anyhow::bail!("chi_squared(df=0) is degenerate");
            }
        }
        T::KolmogorovSmirnov | T::Bootstrap(_) => {
            if r.counts.len() < 2 {
                anyhow::bail!(
                    "test requires at least 2 bins, got {}",
                    r.counts.len()
                );
            }
        }
    }
    Ok(())
}
