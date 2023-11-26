from lib import *

def run_migration(conn):
    print("Running bulk create vertex migration...")
    length = 128
    vertex_creations = []

    for x in range(length):
        for y in range(length):
            for z in range(length):
                vertex_detail = f'''
                    (x{x}y{y}z{z}:Location {{
                        id: '{id()}',
                        name: '({x},{y},{z})',
                        vertex: map(["x", "y", "z"], [{{num:[{x}], str: []}}, {{num:[{y}], str: []}}, {{num:[{z}], str: []}}])
                    }})'''
                vertex_creations.append(vertex_detail)

    combined_query = f"CREATE {', '.join(vertex_creations)}"

    execute(conn, combined_query)
    print("Bulk vertex creation completed.")
