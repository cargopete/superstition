use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use arrow::array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Float32Array, Float64Array, Int16Array,
    Int32Array, Int64Array, Int8Array, LargeBinaryArray, LargeStringArray, StringArray,
    TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
    TimestampSecondArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow::datatypes::{DataType, TimeUnit};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

// ── value / row types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Value {
    U64(u64),
    I64(i64),
    F64(f64),
    Str(String),
    Bytes(Vec<u8>),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub fields: Vec<(String, Value)>,
}

// ── Corpus ────────────────────────────────────────────────────────────────────

/// A content-addressed, read-only collection of parquet tables.
///
/// Each `.parquet` file in the directory is one table (filename sans extension).
/// `corpus_id` is the first 16 hex chars of the BLAKE3 hash of the
/// sorted concatenation of all file bytes — stable across runs.
#[derive(Clone)]
pub struct Corpus {
    pub corpus_id: String,
    tables: HashMap<String, PathBuf>,
}

impl Corpus {
    /// Open a corpus directory.  Errors if the directory doesn't exist.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let mut tables = HashMap::new();
        let mut hasher = blake3::Hasher::new();

        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .with_context(|| format!("opening corpus dir {}", dir.display()))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |x| x == "parquet"))
            .collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in &entries {
            let path = entry.path();
            let table = path.file_stem().unwrap().to_string_lossy().into_owned();
            let bytes = std::fs::read(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            hasher.update(&bytes);
            tables.insert(table, path);
        }

        let corpus_id = hasher.finalize().to_hex()[..16].to_string();
        Ok(Self { corpus_id, tables })
    }

    pub fn has_table(&self, table: &str) -> bool {
        self.tables.contains_key(table)
    }

    /// Read all rows from a parquet table into memory.
    ///
    /// For M3 (test fixture ≤ 100 k rows) this is fine.  M4 will add lazy
    /// streaming for billion-row corpora.
    pub fn rows(&self, table: &str) -> Result<Vec<Row>> {
        let path = self
            .tables
            .get(table)
            .ok_or_else(|| anyhow::anyhow!("no table '{table}' in corpus"))?;

        let file = File::open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
        let reader = builder.build()?;

        let mut rows = Vec::new();
        for batch in reader {
            let batch = batch?;
            rows.extend(batch_to_rows(&batch)?);
        }
        Ok(rows)
    }
}

// ── internal helpers ──────────────────────────────────────────────────────────

fn batch_to_rows(batch: &arrow::record_batch::RecordBatch) -> Result<Vec<Row>> {
    let schema = batch.schema();
    let n = batch.num_rows();
    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        let mut fields = Vec::new();
        for (ci, field) in schema.fields().iter().enumerate() {
            let val = col_value(batch.column(ci), i);
            fields.push((field.name().clone(), val));
        }
        rows.push(Row { fields });
    }
    Ok(rows)
}

fn col_value(col: &ArrayRef, i: usize) -> Value {
    if col.is_null(i) {
        return Value::Null;
    }
    match col.data_type() {
        DataType::UInt8 => Value::U64(col.as_any().downcast_ref::<UInt8Array>().unwrap().value(i) as u64),
        DataType::UInt16 => Value::U64(col.as_any().downcast_ref::<UInt16Array>().unwrap().value(i) as u64),
        DataType::UInt32 => Value::U64(col.as_any().downcast_ref::<UInt32Array>().unwrap().value(i) as u64),
        DataType::UInt64 => Value::U64(col.as_any().downcast_ref::<UInt64Array>().unwrap().value(i)),
        DataType::Int8 => Value::I64(col.as_any().downcast_ref::<Int8Array>().unwrap().value(i) as i64),
        DataType::Int16 => Value::I64(col.as_any().downcast_ref::<Int16Array>().unwrap().value(i) as i64),
        DataType::Int32 => Value::I64(col.as_any().downcast_ref::<Int32Array>().unwrap().value(i) as i64),
        DataType::Int64 => Value::I64(col.as_any().downcast_ref::<Int64Array>().unwrap().value(i)),
        DataType::Float32 => Value::F64(col.as_any().downcast_ref::<Float32Array>().unwrap().value(i) as f64),
        DataType::Float64 => Value::F64(col.as_any().downcast_ref::<Float64Array>().unwrap().value(i)),
        DataType::Utf8 => Value::Str(col.as_any().downcast_ref::<StringArray>().unwrap().value(i).to_string()),
        DataType::LargeUtf8 => Value::Str(col.as_any().downcast_ref::<LargeStringArray>().unwrap().value(i).to_string()),
        DataType::Binary => Value::Bytes(col.as_any().downcast_ref::<BinaryArray>().unwrap().value(i).to_vec()),
        DataType::LargeBinary => Value::Bytes(col.as_any().downcast_ref::<LargeBinaryArray>().unwrap().value(i).to_vec()),
        DataType::Boolean => Value::Bool(col.as_any().downcast_ref::<BooleanArray>().unwrap().value(i)),
        DataType::Timestamp(TimeUnit::Second, _) => {
            Value::U64(col.as_any().downcast_ref::<TimestampSecondArray>().unwrap().value(i) as u64)
        }
        DataType::Timestamp(TimeUnit::Millisecond, _) => {
            Value::U64((col.as_any().downcast_ref::<TimestampMillisecondArray>().unwrap().value(i) / 1_000) as u64)
        }
        DataType::Timestamp(TimeUnit::Microsecond, _) => {
            Value::U64((col.as_any().downcast_ref::<TimestampMicrosecondArray>().unwrap().value(i) / 1_000_000) as u64)
        }
        DataType::Timestamp(TimeUnit::Nanosecond, _) => {
            Value::U64((col.as_any().downcast_ref::<TimestampNanosecondArray>().unwrap().value(i) / 1_000_000_000) as u64)
        }
        _ => Value::Null,
    }
}
