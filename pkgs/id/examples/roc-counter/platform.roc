platform ""
    requires {
        [Model : model] for program : {
            init : U64 -> model,
            update : model, Str -> model,
            view : model, Str -> Str,
            records : model -> Str,
        }
    }
    exposes []
    packages {}
    provides {
        "roc_init": init_for_host,
        "roc_update": update_for_host,
        "roc_view": view_for_host,
        "roc_records": records_for_host,
    }
    targets: {
        inputs_dir: "targets/",
        wasm32: { inputs: ["host.wasm", app], output: Shared, exports: ["plaza_alloc", "plaza_free", "plaza_init", "plaza_update", "plaza_view", "plaza_records", "plaza_out_len", "plaza_error_ptr", "plaza_error_len"] },
    }

init_for_host : U64 -> Box(Model)
init_for_host = |seed| Box.box((program.init)(seed))

update_for_host : Box(Model), Str -> Box(Model)
update_for_host = |boxed, ev| Box.box((program.update)(Box.unbox(boxed), ev))

view_for_host : Box(Model), Str -> Str
view_for_host = |boxed, viewer| (program.view)(Box.unbox(boxed), viewer)

records_for_host : Box(Model) -> Str
records_for_host = |boxed| (program.records)(Box.unbox(boxed))
