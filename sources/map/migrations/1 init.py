from lib import *

def run_migration(conn):
    print("Running 1 initial test migration...")

    execute(conn, "CREATE NODE TABLE User(name STRING, age UINT64, PRIMARY KEY (name))")
    execute(conn, "CREATE NODE TABLE City(name STRING, population UINT64, PRIMARY KEY (name))")
    execute(conn, "CREATE REL TABLE Follows(FROM User TO User, since UINT64)")
    execute(conn, "CREATE REL TABLE LivesIn(FROM User TO City)")
