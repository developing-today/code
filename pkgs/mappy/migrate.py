from lib import *

data_directory = "./data"
migrations_directory = "./migrations"

db = kuzu.Database(data_directory)
conn = kuzu.Connection(db)

print_call_info(conn)

first_run = True
try:
    version, error = get_latest_version(conn)

    if version is None:
        create_version_zero(conn)

        version, error = get_latest_version(conn)

        if version is None and error == "":
            raise Exception("Version not found")
        elif version is None and error != "":
            raise Exception(error)
        elif version[1] != 0:
            first_run = False
            print("Version:", version)

            if error != "":
                raise Exception(error)
    else:
        first_run = False
        print("Version:", version)

        if error != "":
            raise Exception(error)

except Exception as e:
    if first_run:
        create_version_table(conn)
        create_version_zero(conn)
        version, error = get_latest_version(conn)
        if error != "" and version is not None and version[1] != 0:
            raise Exception(error) from e
    else:
        raise
if version is None:
    raise Exception("Version not found")

migrations = sort_migrations(migrations_directory)

if len(migrations) <= version[1]:
    print("No new migrations found")
else:
    run_migrations(conn, migrations_directory, migrations, version[1])
version = get_latest_version(conn)

print("Version:", version)
