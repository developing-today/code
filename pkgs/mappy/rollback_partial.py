from lib import *

data_directory = "./data"
migrations_directory = "./migrations"

db = kuzu.Database(data_directory)
conn = kuzu.Connection(db)

print_call_info(conn)

version, error = get_latest_version(conn)

if version is None:
    raise Exception("Version not found")
else:
    print("Version:", version)

    if version[2] > 0:
        raise Exception("Version is already complete")

    if error == "":
        raise Exception("Version is not in error")

    execute(
        conn,
        f"""
        MATCH (v:Version)
        WHERE v.counter = {version[1]}
        DELETE v return v.*
    """,
    )
