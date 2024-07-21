from lib import *


def run_migration(conn):
    print("Running 2 update migration...")

    execute(
        conn,
        f"""
      CREATE NODE TABLE Location(
        id STRING,
        name STRING,
        vertex MAP(string,STRUCT(num INT64[], str STRING[])),
        properties MAP(string, STRUCT(num INT64[], str STRING[])),
        admin MAP(string, STRUCT(num INT64[], str STRING[])),
        acl MAP(string, STRUCT(num INT64[], str STRING[])),
        created MAP(TIMESTAMP, STRUCT(num INT64[], str STRING[])),
        updated MAP(TIMESTAMP, STRUCT(num INT64[], str STRING[])),
        claimed MAP(TIMESTAMP, STRUCT(num INT64[], str STRING[])),
        owners MAP(TIMESTAMP, STRUCT(num INT64[], str STRING[])),
        PRIMARY KEY (id))
      """,
    )

    execute(
        conn, "CREATE REL TABLE Neighbors(FROM Location TO Location, distance UINT64)"
    )
