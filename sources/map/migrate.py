from lib import *

db = kuzu.Database('./data')
conn = kuzu.Connection(db)

try:
    version = get_latest_version(conn)

    if version is None:
        create_version_zero(conn)

    version = get_latest_version(conn)

except:
    create_version_table(conn)
    create_version_zero(conn)
    version = get_latest_version(conn)

if version is None:
    raise Exception("Version not found")

migrations_directory = './migrations'
migrations = sorted([f.name for f in os.scandir(migrations_directory) if f.is_file() and f.name.endswith('.py')])

if len(migrations) <= version[1]:
    print("No new migrations found")
else:
    run_migrations(conn, migrations_directory, migrations, version[1])
version = get_latest_version(conn)

print("Version:", version)
