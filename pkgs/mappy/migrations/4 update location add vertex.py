from lib import *


def run_migration(conn):
    print("Running 4 update add vertex migration...")

    this_location_id = id()

    execute(
        conn,
        f"""
      CREATE (l:Location {{
        id: '{this_location_id}'
      }})
    """,
    )

    execute(
        conn,
        f"""
        MATCH (l:Location)
        WHERE l.id = '{this_location_id}'
        SET
          l.name = 'Location Vertex',
          l.vertex = map(["x", "y", "z"], [{{num:cast([], 'INT64[]'), str: []}}, {{num:cast([], 'INT64[]'), str: []}}, {{num:cast([], 'INT64[]'), str: []}}])
    """,
    )
