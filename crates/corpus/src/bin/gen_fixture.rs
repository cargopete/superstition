//! Generate a synthetic `erc20_transfers` parquet fixture for M3 testing.
//!
//! Usage:  cargo run -p superstition-corpus --bin gen-fixture -- [output_dir]
//!
//! Default output: `corpus/test_fixture/`
//!
//! The fixture has 10,000 rows with a non-uniform day-of-week distribution:
//!
//!   Mon=2000  Tue=1480  Wed=1480  Thu=1480  Fri=1480  Sat=1480  Sun=600
//!
//! This gives chi-squared ≈ 718 (df=6), Cramér's V ≈ 0.11, p ≈ 0.
//! The dow-erc20 detector should report SIGNIFICANT with this corpus.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use arrow::array::UInt64Array;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

fn main() -> Result<()> {
    let out_dir =
        PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "corpus/test_fixture".to_string()));
    fs::create_dir_all(&out_dir)?;

    let out_path = out_dir.join("erc20_transfers.parquet");

    // epoch_day 19355 has epoch_day % 7 == 0.
    // day_index = (epoch_day + 4) % 7  →  0=Sun 1=Mon 2=Tue 3=Wed 4=Thu 5=Fri 6=Sat
    //
    //  epoch_day % 7 == 0  →  day_index 4  (Thursday)
    //  epoch_day % 7 == 1  →  day_index 5  (Friday)
    //  epoch_day % 7 == 2  →  day_index 6  (Saturday)
    //  epoch_day % 7 == 3  →  day_index 0  (Sunday)
    //  epoch_day % 7 == 4  →  day_index 1  (Monday)
    //  epoch_day % 7 == 5  →  day_index 2  (Tuesday)
    //  epoch_day % 7 == 6  →  day_index 3  (Wednesday)
    //
    // target counts by epoch_day_mod:
    //   mod 4 (Mon) = 2000,  mod 2 (Sat) = 1480,  mod 0 (Thu) = 1480,
    //   mod 1 (Fri) = 1480,  mod 5 (Tue) = 1480,  mod 6 (Wed) = 1480,
    //   mod 3 (Sun) = 600
    const BASE_EPOCH_DAY: u64 = 19355; // 19355 % 7 == 0

    let targets: &[(u64, u64)] = &[
        (4, 2000), // Monday
        (5, 1480), // Tuesday
        (6, 1480), // Wednesday
        (0, 1480), // Thursday
        (1, 1480), // Friday
        (2, 1480), // Saturday
        (3, 600),  // Sunday
    ];

    let mut timestamps: Vec<u64> = Vec::with_capacity(10_000);

    for &(day_mod, count) in targets {
        // First epoch_day on or after BASE_EPOCH_DAY with the right mod.
        // Since BASE_EPOCH_DAY % 7 == 0:  base_day = BASE_EPOCH_DAY + day_mod
        let base_day = BASE_EPOCH_DAY + day_mod;

        // Spread `count` rows across 52 weeks, multiple per week if needed.
        let num_weeks: u64 = 52;
        let per_week = count / num_weeks;
        let extra = count % num_weeks;

        for week in 0..num_weeks {
            let epoch_day = base_day + week * 7;
            let n = if week < extra { per_week + 1 } else { per_week };
            for j in 0..n {
                // Spread within the day to avoid identical timestamps.
                let secs_in_day = if n > 1 { j * 86_400 / n } else { 0 };
                timestamps.push(epoch_day * 86_400 + secs_in_day);
            }
        }
    }

    // Sort by timestamp so the parquet file is naturally ordered.
    timestamps.sort_unstable();

    // ── write parquet ──
    let schema = Arc::new(Schema::new(vec![Field::new(
        "block_timestamp",
        DataType::UInt64,
        false,
    )]));

    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();

    let file = fs::File::create(&out_path)?;
    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))?;

    for chunk in timestamps.chunks(1_000) {
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(UInt64Array::from(chunk.to_vec()))],
        )?;
        writer.write(&batch)?;
    }
    writer.close()?;

    println!("wrote {} rows → {}", timestamps.len(), out_path.display());
    println!("run with:  cargo run -p superstition-host -- --corpus {} <detector.wasm>",
        out_dir.display());
    Ok(())
}
