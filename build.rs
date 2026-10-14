// Builds SkateGM's texture encoder (src/vtfenc.rs, copied verbatim from their repo) as a library for the
// target, so gmtoolkit can hand it to their map builder (see `install_skategm`). Their release zip ships only
// the source, and maps must encode textures identically on every PC. If it can't be built (no linker for the
// target), gmtoolkit falls back to building it with the user's rustc at install time.
use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo::rerun-if-changed=src/vtfenc.rs");
    println!("cargo::rustc-check-cfg=cfg(vtfenc)");
    let target = env::var("TARGET").unwrap();
    // SkateGM's module is win64-only: Windows, or Linux running GMod under Proton.
    if !target.starts_with("x86_64-") || !(target.contains("windows") || target.contains("linux")) {
        return;
    }
    let dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let out = dir.join("vtfenc");
    // Their one libm call, powf(2.2), rounds differently per platform (glibc, MS ucrt and MinGW disagree by up to
    // 4 ULP), which can change a texture's reflectivity and so the map. Swap in the exact values their Setup.exe
    // encoder (MinGW build) computes, so ours encode like every Windows player's.
    // ponytail: until SkateGM replaces the powf with a table upstream; then drop this and the table.
    let source = fs::read_to_string("src/vtfenc.rs").unwrap();
    let call = "(v as f64 / 255.0).powf(2.2)";
    let patched = if source.contains(call) {
        source.replacen(call, "f64::from_bits(SETUP_POW[v as usize])", 1) + SETUP_POW
    } else {
        println!("cargo::warning=vtfenc.rs no longer has the powf call: building it unpatched");
        source
    };
    fs::write(dir.join("vtfenc.rs"), patched).unwrap();
    let mut cmd = Command::new(env::var("RUSTC").unwrap());
    cmd.args(["--edition", "2024", "--crate-type", "cdylib", "-C", "opt-level=3", "-C", "panic=abort", "-C", "strip=symbols"])
        .args(["--target", &target]).arg(dir.join("vtfenc.rs")).arg("-o").arg(&out);
    // The target's linker and flags (static CRT on Windows, cargo-xwin's library paths when cross-building).
    if let Ok(linker) = env::var("RUSTC_LINKER") { cmd.arg(format!("-Clinker={linker}")); }
    let flags = env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    cmd.args(flags.split('\x1f').filter(|f| !f.is_empty()));
    match cmd.status() {
        Ok(s) if s.success() && out.is_file() => {
            // For main.rs's .incbin: forward slashes, as backslashes are escapes in assembler strings.
            let path = |p: PathBuf| p.to_string_lossy().replace('\\', "/");
            println!("cargo::rustc-env=VTFENC_LIB={}", path(out));
            println!("cargo::rustc-env=VTFENC_SRC={}", path(PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("src/vtfenc.rs")));
            println!("cargo::rustc-cfg=vtfenc");
        }
        _ => println!("cargo::warning=couldn't build SkateGM's texture encoder; installs will need rustc for maps"),
    }
}

/// pow(v / 255, 2.2) for v in 0..=255 as SkateGM 9.0's Setup.exe vtfenc.dll computes it (MinGW's pow, called in
/// that DLL under Wine), as f64 bits.
const SETUP_POW: &str = "
const SETUP_POW: [u64; 256] = [
    0x0000000000000000, 0x3ed54b71665fd807, 0x3ef8761046218a58, 0x3f0dd7e8fa315a11, 0x3f1c1938dcfab695, 0x3f26f42f832248a3,
    0x3f3123f9df4c5aa9, 0x3f380f8645a41461, 0x3f40236bd45ecf29, 0x3f44e97737ce0d27, 0x3f4a5df6f84d3c1a, 0x3f504251cb9be4bd,
    0x3f53b075fc9ea7f2, 0x3f577b008c60c7e6, 0x3f5ba36f87788731, 0x3f6015950d996312, 0x3f6289c1ba29e4c3, 0x3f652ede8bceba92,
    0x3f68058469b058fd, 0x3f6b0e453058edfb, 0x3f6e49ac62759424, 0x3f70dc1fe000cb2d, 0x3f72ad3fe45204f8, 0x3f749874167a2da0,
    0x3f769df818c73647, 0x3f78be058322d633, 0x3f7af8d409a8ed78, 0x3f7d4e999efb2137, 0x3f7fbf8a92ef02d2, 0x3f8125ecd70c3a38,
    0x3f8279dc254e02b2, 0x3f83dbab3551b0ed, 0x3f854b71665fd80a, 0x3f86c9457f3d0c1c, 0x3f88553db6a4958a, 0x3f89ef6fbb0cfeb6,
    0x3f8b97f0b9caa096, 0x3f8d4ed565a19714, 0x3f8f1431fcd63948, 0x3f90740d2764a0ad, 0x3f916550e0959446, 0x3f925dedaa686ae6,
    0x3f935decd5194421, 0x3f94655782ce6f47, 0x3f957436a98d7b8f, 0x3f968a93150ff02b, 0x3f97a875687a7112, 0x3f98cde61ff8cb14,
    0x3f99faed92411f15, 0x3f9b2f93f2002c08, 0x3f9c6be14f30830c, 0x3f9dafdd985e45d4, 0x3f9efb909bd8e67a, 0x3fa02781046a1f11,
    0x3fa0d51cb83d1798, 0x3fa1869f2376f6c7, 0x3fa23c0bf220f668, 0x3fa2f566c2b0afe2, 0x3fa3b2b326770f86, 0x3fa473f4a209de03,
    0x3fa5392eada83a12, 0x3fa60264b59a554a, 0x3fa6cf9a1a8cc07e, 0x3fa7a0d231e78ded, 0x3fa8761046218a58, 0x3fa94f57970fc8c8,
    0x3faa2cab5a31b8ef, 0x3fab0e0ebaf9fb28, 0x3fabf384db142257, 0x3facdd10d2a78f7f, 0x3fadcab5b09790d4, 0x3faebc767ac0ea0c,
    0x3fafb2562e34ea53, 0x3fb0562bdfb91872, 0x3fb0d53f0d4d9fad, 0x3fb1566611d57b8b, 0x3fb1d9a25b538e70, 0x3fb25ef553e87e14,
    0x3fb2e66061ea20ed, 0x3fb36fe4e7fa129e, 0x3fb3fb84451b79f4, 0x3fb4893fd4c80a38, 0x3fb51918ef044945, 0x3fb5ab10e87322fe,
    0x3fb63f291268d28d, 0x3fb6d562bafd28f2, 0x3fb76dbf2d1d3875, 0x3fb8083fb09c6ba6, 0x3fb8a4e58a450eaf, 0x3fb943b1fbe850e3,
    0x3fb9e4a6446dc49c, 0x3fba87c39fe262ab, 0x3fbb2d0b478716e9, 0x3fbbd47e71deda80, 0x3fbc7e1e52bc60e2, 0x3fbd29ec1b4f5bb2,
    0x3fbdd7e8fa315a11, 0x3fbe88161b724718, 0x3fbf3a74a8a48b67, 0x3fbfef05c8e8d57a, 0x3fc052e5507cc60f, 0x3fc0af62299af72a,
    0x3fc10cf9ffd671f0, 0x3fc16bad6213c08f, 0x3fc1cb7cde19a0d1, 0x3fc22c690095fdca, 0x3fc28e725522c76f, 0x3fc2f199664aa938,
    0x3fc355debd8da113, 0x3fc3bb42e3657795, 0x3fc421c65f4a1ab8, 0x3fc48969b7b5dbfb, 0x3fc4f22d7229930b, 0x3fc55c121330a5bd,
    0x3fc5c7181e64f662, 0x3fc633401672b92a, 0x3fc6a08a7d1c31a1, 0x3fc70ef7d33d58d9, 0x3fc77e8898cf6d21, 0x3fc7ef3d4cec6bfe,
    0x3fc861166dd27714, 0x3fc8d41478e724be, 0x3fc94837eababcd4, 0x3fc9bd813f0b6253, 0x3fca33f0f0c82a9d, 0x3fcaab877a1422a6,
    0x3fcb2445544942c2, 0x3fcb9e2af7fb5191, 0x3fcc1938dcfab69a, 0x3fcc956f7a573cf6, 0x3fcd12cf4662c68f, 0x3fcd9158b6b3f076,
    0x3fce110c4028a8b3, 0x3fce91ea56e8b5db, 0x3fcf13f36e683135, 0x3fcf9727f969f345, 0x3fd00dc43500f9d1, 0x3fd0508a98cbce0b,
    0x3fd093e7607407cf, 0x3fd0d7dac40432bb, 0x3fd11c64fb33ec59, 0x3fd161863d68f632, 0x3fd1a73ec1b8428f, 0x3fd1ed8ebee6fbcb,
    0x3fd234766b6b867e, 0x3fd27bf5fd6e7ea4, 0x3fd2c40daacbafd7, 0x3fd30cbda91308bf, 0x3fd356062d8989df, 0x3fd39fe76d2a2fcf,
    0x3fd3ea619ca6d91d, 0x3fd43574f06927ce, 0x3fd481219c935eaf, 0x3fd4cd67d5013a94, 0x3fd51a47cd48c798, 0x3fd567c1b8bb3272,
    0x3fd5b5d5ca659609, 0x3fd604843511c543, 0x3fd653cd2b47114f, 0x3fd6a3b0df4b0c51, 0x3fd6f42f832248a9, 0x3fd74549489114de,
    0x3fd796fe611c3439, 0x3fd7e94efe099438, 0x3fd83c3b5060fed6, 0x3fd88fc388ecc9c4, 0x3fd8e3e7d83a82b6, 0x3fd938a86e9b98b3,
    0x3fd98e057c260290, 0x3fd9e3ff30b4e2ab, 0x3fda3a95bbe927e1, 0x3fda91c94d2a2be5, 0x3fdae99a13a64ef6, 0x3fdb42083e539109,
    0x3fdb9b13fbf02881, 0x3fdbf4bd7b03166c, 0x3fdc4f04e9dcb861, 0x3fdca9ea76975813, 0x3fdd056e4f17b88b, 0x3fdd6190a10da13b,
    0x3fddbe5199f466da, 0x3fde1bb16713720b, 0x3fde79b0357ec416, 0x3fded84e3217796f, 0x3fdf378b898c4a4a, 0x3fdf9768685a093c,
    0x3fdff7e4facc1fec, 0x3fe02c80b67e84ef, 0x3fe05d5ef56b66b7, 0x3fe08e8d5009b977, 0x3fe0c00bdc1ebd0b, 0x3fe0f1daaf5826d4,
    0x3fe123f9df4c5aac, 0x3fe15669817aa310, 0x3fe18929ab4b686f, 0x3fe1bc3a721067b1, 0x3fe1ef9beb04e7f7, 0x3fe2234e2b4def8b,
    0x3fe2575147fa781d, 0x3fe28ba55603a23d, 0x3fe2c04a6a4ce811, 0x3fe2f54099a44f66, 0x3fe32a87f8c29b04, 0x3fe360209c4b7b4e,
    0x3fe3960a98cdbe44, 0x3fe3cc4602c37ec8, 0x3fe402d2ee925356, 0x3fe439b1708b7c03, 0x3fe470e19cec0fe6, 0x3fe4a86387dd29e4,
    0x3fe4e037457414e2, 0x3fe5185ce9b2775c, 0x3fe550d488867e6c, 0x3fe5899e35cb0837, 0x3fe5c2ba0547cddc, 0x3fe5fc280ab18cc4,
    0x3fe635e859aa2f6d, 0x3fe66ffb05c0f5b6, 0x3fe6aa6022729c97, 0x3fe6e517c3298565, 0x3fe72021fb3ddc89, 0x3fe75b7eddf5bfbb,
    0x3fe7972e7e8563ce, 0x3fe7d330f00f39ea, 0x3fe80f8645a41462, 0x3fe84c2e92434b0b, 0x3fe88929e8dadf20, 0x3fe8c6785c479eb3,
    0x3fe90419ff5547b3, 0x3fe9420ee4beaa78, 0x3fe980571f2dcbf4, 0x3fe9bef2c13c0768, 0x3fe9fde1dd722fbc, 0x3fea3d248648b067,
    0x3fea7cbace27adfb, 0x3feabca4c7672641, 0x3feafce2844f1003, 0x3feb3d7417177a5e, 0x3feb7e5991e8abd2, 0x3febbf9306db40db,
    0x3fec012087f84a3b, 0x3fec430227396ae8, 0x3fec8537f688f59c, 0x3fecc7c207c20a12, 0x3fed0aa06cb0b1ec, 0x3fed4dd33711fd3c,
    0x3fed915a78941ecc, 0x3fedd53642d687fd, 0x3fee1966a76a0465, 0x3fee5debb7d0d513, 0x3feea2c5857ecb8d, 0x3feee7f421d9647b,
    0x3fef2d779e37e205, 0x3fef73500be365e5, 0x3fefb97d7c170b39, 0x3ff0000000000000,
];
";
