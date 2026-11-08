//! Build script that generates FFmpeg bindings and configures native linking.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn query(package: &str, variable: &str) -> String {
    let output = Command::new("pkg-config")
        .arg(format!("--variable={variable}"))
        .arg(package)
        .output()
        .expect("FFmpeg API export requires pkg-config and FFmpeg development libraries");
    assert!(
        output.status.success(),
        "Missing {package}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = String::from_utf8(output.stdout).expect("pkg-config path must be UTF-8");
    let value = value.trim();
    assert!(
        !value.is_empty(),
        "pkg-config returned no {variable} for {package}"
    );
    value.to_owned()
}

fn main() {
    println!("cargo:rerun-if-changed=src/wrapper.h");
    for variable in [
        "LIBCLANG_PATH",
        "BINDGEN_EXTRA_CLANG_ARGS",
        "PKG_CONFIG_PATH",
        "PKG_CONFIG_LIBDIR",
        "PKG_CONFIG_SYSROOT_DIR",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut bindings = bindgen::Builder::default()
        .header("src/wrapper.h")
        .allowlist_type("SwsFlags|MotionFfmpegConstants")
        .allowlist_var("AVFMT_GLOBALHEADER|AVIO_FLAG_WRITE|AV_CODEC_FLAG_GLOBAL_HEADER|SWS_BICUBIC|SWS_CS_ITU709|AV_DICT_IGNORE_SUFFIX")
        .allowlist_function("sws_getCoefficients|sws_setColorspaceDetails")
        .prepend_enum_name(false)
        .rust_edition(bindgen::RustEdition::Edition2024)
        .generate_comments(false)
        .layout_tests(false)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));
    for function in [
        "avutil_version",
        "av_strerror",
        "av_opt_set",
        "av_channel_layout_default",
        "avformat_(alloc_output_context2|new_stream|write_header|free_context|open_input|close_input|find_stream_info)",
        "av_find_best_stream",
        "av_read_frame",
        "av_interleaved_write_frame",
        "av_write_trailer",
        "avio_(open|closep)",
        "av_dict_(set|free|count|get)",
        "av_calloc",
        "avcodec_(find_encoder_by_name|find_decoder|alloc_context3|free_context|open2|send_frame|receive_packet|send_packet|receive_frame|parameters_from_context|parameters_to_context)",
        "av_frame_(alloc|free|unref|get_buffer|make_writable)",
        "av_packet_(alloc|free|unref|rescale_ts)",
        "avfilter_graph_(alloc|free|parse2|create_filter|alloc_filter|config)",
        "avfilter_(get_by_name|init_dict|link|inout_free)",
        "av_buffersink_get_(format|sample_rate|channels|samples)",
        "sws_(getContext|scale|freeContext)",
    ] {
        bindings = bindings.allowlist_function(function);
    }
    let mut link_directories = std::collections::BTreeSet::new();
    for package in [
        "libavformat",
        "libavcodec",
        "libavfilter",
        "libswscale",
        "libswresample",
        "libavutil",
    ] {
        bindings = bindings.clang_arg(format!("-I{}", query(package, "includedir")));
        let directory = query(package, "libdir");
        if link_directories.insert(directory.clone()) {
            println!("cargo:rustc-link-search=native={directory}");
            if env::var("CARGO_CFG_TARGET_FAMILY").as_deref() == Ok("unix") {
                println!("cargo:rustc-link-arg=-Wl,-rpath,{directory}");
            }
        }
        println!(
            "cargo:rustc-link-lib={}",
            package.strip_prefix("lib").unwrap()
        );
    }
    bindings
        .generate()
        .expect("Cannot generate FFmpeg bindings; install libclang and FFmpeg development headers")
        .write_to_file(output.join("bindings.rs"))
        .expect("Cannot write generated FFmpeg bindings");
}
