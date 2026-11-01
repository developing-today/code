platform ""
    requires {
        [Model : model] for program : {
            init : U64 -> model,
            update : model, U64, Str -> model,
            view : model -> Str,
            layout : model, U64, U64 -> Str,
            records : model -> Str,
            wants : model -> Str,
            snapshot : model -> Str,
            restore : Str -> model,
        }
    }
    exposes [Screen, Key, RocJson, RocGrid, RocCap, RocText]
    packages {}
    provides {
        "roc_init": init_for_host,
        "roc_update": update_for_host,
        "roc_view": view_for_host,
        "roc_layout": layout_for_host,
        "roc_records": records_for_host,
        "roc_wants": wants_for_host,
        "roc_snapshot": snapshot_for_host,
        "roc_restore": restore_for_host,
    }
    targets: {
        inputs_dir: "targets/",
        x64musl: { inputs: ["crt1.o", "libhost.a", app, "libc.a", "libzigc.a", "libcompiler_rt.a"] },
        wasm32: { inputs: ["host.wasm", app], output: Shared, exports: ["plaza_abi_version", "plaza_alloc", "plaza_free", "plaza_init", "plaza_update", "plaza_view", "plaza_layout", "plaza_records", "plaza_wants", "plaza_snapshot", "plaza_restore", "plaza_out_len", "plaza_error_ptr", "plaza_error_len"] },
    }

import Screen
import Key
import RocJson
import RocGrid
import RocCap
import RocText

init_for_host : U64 -> Box(Model)
init_for_host = |seed| Box.box((program.init)(seed))

update_for_host : Box(Model), U64, Str -> Box(Model)
update_for_host = |boxed, pid, ev| Box.box((program.update)(Box.unbox(boxed), pid, ev))

view_for_host : Box(Model) -> Str
view_for_host = |boxed| (program.view)(Box.unbox(boxed))

layout_for_host : Box(Model), U64, U64 -> Str
layout_for_host = |boxed, width, height| (program.layout)(Box.unbox(boxed), width, height)

records_for_host : Box(Model) -> Str
records_for_host = |boxed| (program.records)(Box.unbox(boxed))

## What the program wants from the server: subscriptions and one-shot
## requests, as the JSON document of the world's capability protocol.
wants_for_host : Box(Model) -> Str
wants_for_host = |boxed| (program.wants)(Box.unbox(boxed))

snapshot_for_host : Box(Model) -> Str
snapshot_for_host = |boxed| (program.snapshot)(Box.unbox(boxed))

restore_for_host : Str -> Box(Model)
restore_for_host = |text| Box.box((program.restore)(text))
