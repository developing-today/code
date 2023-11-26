from lib import *

def run_migration(conn):
    print("Running update to create Neighbors relationships...")

    query = """
    MATCH (v1:Location), (v2:Location)
    WHERE
        map_extract(v1.vertex, 'x') IS NOT NULL AND
        map_extract(v2.vertex, 'x') IS NOT NULL AND
        size(map_extract(v1.vertex, 'x')) > 0 AND
        size(map_extract(v2.vertex, 'x')) > 0 AND
        size(struct_extract(map_extract(v1.vertex, 'x')[1], 'num')) > 0 AND
        size(struct_extract(map_extract(v2.vertex, 'x')[1], 'num')) > 0 AND
        abs(struct_extract(map_extract(v1.vertex, 'x')[1], 'num')[1] - struct_extract(map_extract(v2.vertex, 'x')[1], 'num')[1]) <= 1 AND

        map_extract(v1.vertex, 'y') IS NOT NULL AND
        map_extract(v2.vertex, 'y') IS NOT NULL AND
        size(map_extract(v1.vertex, 'y')) > 0 AND
        size(map_extract(v2.vertex, 'y')) > 0 AND
        size(struct_extract(map_extract(v1.vertex, 'y')[1], 'num')) > 0 AND
        size(struct_extract(map_extract(v2.vertex, 'y')[1], 'num')) > 0 AND
        abs(struct_extract(map_extract(v1.vertex, 'y')[1], 'num')[1] - struct_extract(map_extract(v2.vertex, 'y')[1], 'num')[1]) <= 1 AND

        map_extract(v1.vertex, 'z') IS NOT NULL AND
        map_extract(v2.vertex, 'z') IS NOT NULL AND
        size(map_extract(v1.vertex, 'z')) > 0 AND
        size(map_extract(v2.vertex, 'z')) > 0 AND
        size(struct_extract(map_extract(v1.vertex, 'z')[1], 'num')) > 0 AND
        size(struct_extract(map_extract(v2.vertex, 'z')[1], 'num')) > 0 AND
        abs(struct_extract(map_extract(v1.vertex, 'z')[1], 'num')[1] - struct_extract(map_extract(v2.vertex, 'z')[1], 'num')[1]) <= 1

    CREATE (v1)-[:Neighbors]->(v2)
    """

    execute(conn, query)
    print("Neighbors relationships creation completed.")
