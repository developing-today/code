from lib import *

def run_migration(conn):
    print("Running initial migration...")

    execute(conn, "CREATE NODE TABLE User(name STRING, age INT64, PRIMARY KEY (name))")
    execute(conn, "CREATE NODE TABLE City(name STRING, population INT64, PRIMARY KEY (name))")
    execute(conn, "CREATE REL TABLE Follows(FROM User TO User, since INT64)")
    execute(conn, "CREATE REL TABLE LivesIn(FROM User TO City)")
