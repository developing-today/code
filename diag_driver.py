import importlib.util, json
spec = importlib.util.spec_from_file_location("t", "pkgs/cold-store-reaper/tests/test_turso_mesh_restore_verify.py")
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
case = m.RestoreVerifyAcceptance("test_all_three_pass_against_manifest_with_queue_query")
case.setUp()
try:
    report = case.run_verify()
    for r in report["databases"]:
        print("DBREPORT " + json.dumps(r)[:230])
finally:
    case.tearDown()
