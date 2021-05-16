#![cfg(any(target_arch = "x86", target_arch = "x86_64"))]

mod engine;
mod set;
mod single_meth;

use engine::Section;
use proj_core::SimdRow;
use proj_core::{Row, Stage};

use crate::single_meth::near_calls;

mod tables {
    #![allow(dead_code)]
    use crate::single_meth::far_calls;

    use super::*;

    pub fn yorkshire_s10() -> single_meth::Table<Row> {
        single_meth::Table::from_place_not(
            Stage::ROYAL,
            "x30x14x50x16x1270x38x14x50x16x90,12",
            // Fix the treble and all the tenors
            "17890",
            &near_calls(Stage::ROYAL),
            "LBTFVXSMWH",
        )
        .unwrap()
    }

    pub fn bristol_s8() -> single_meth::Table<Row> {
        single_meth::Table::from_place_not(
            Stage::MAJOR,
            "x58x14.58x58.36.14x14.58x14x18,18",
            "178",
            &near_calls(Stage::MAJOR),
            "LIBMFHVW",
        )
        .unwrap()
    }

    pub fn cambs_s8() -> single_meth::Table<Row> {
        single_meth::Table::from_place_not(
            Stage::MAJOR,
            "-38-14-1258-36-14-58-16-78,12",
            "178",
            &near_calls(Stage::MAJOR)[..1],
            "LBTFVMWH",
        )
        .unwrap()
    }

    pub fn yorkshire_s8() -> single_meth::Table<Row> {
        single_meth::Table::from_place_not(
            Stage::MAJOR,
            "x38x14x58x16x12x38x14x78,12",
            "178",
            &near_calls(Stage::MAJOR),
            "LBTFVMWH",
        )
        .unwrap()
    }
}

fn main() {
    let table = tables::bristol_s8();

    table.print_falseness();

    if SimdRow::are_cpu_features_enabled() {
        println!("Can use SIMD!");
        single_meth::Section::compose(&table.change_row_type::<SimdRow>(), 1280..=1280);
    } else {
        println!("Can't use SIMD.");
        single_meth::Section::compose(&table, 5000..=5184);
    }
}
