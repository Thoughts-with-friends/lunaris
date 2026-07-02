use std::path::{Path, PathBuf};

use criterion::{criterion_group, criterion_main, Criterion};
use nds_core::NDS;

fn bench(c: &mut Criterion) {
    let rom_path = Path::new("ROMs/game.nds");
    let bios7_path = PathBuf::from("ROMs/bios7.bin");
    let bios9_path = PathBuf::from("ROMs/bios9.bin");
    let firmware_path = PathBuf::from("ROMs/firmware.bin");

    let bios7_path = bios7_path.exists().then_some(bios7_path);
    let bios7_path = bios7_path.as_deref();
    let bios9_path = bios9_path.exists().then_some(bios9_path);
    let bios9_path = bios9_path.as_deref();
    let firmware_path = firmware_path.exists().then_some(firmware_path);
    let firmware_path = firmware_path.as_deref();

    c.bench_function("FirstSecond", |b| {
        b.iter_batched(
            || NDS::load_rom(bios7_path, bios9_path, firmware_path, rom_path),
            |mut nds| {
                for _ in 0..60 {
                    nds.emulate_frame();
                }
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
