import math

from lib import *
from concurrent.futures import ThreadPoolExecutor


def run_migration(conn):
    run_migration_parallel(conn)


def run_migration_parallel(conn, **options):
    time_now = datetime.datetime.now()
    length = options.get("length", 128)
    total_parts = options.get("total_parts", 2048)
    max_workers = options.get("max_workers", 2048)

    print(
        f"START | Running update to create {length}^3={length*length*length} vertices. |"
    )

    with ThreadPoolExecutor(max_workers=max_workers) as executor:
        futures = [
            executor.submit(
                run_migration_segment, conn, part, total_parts, length, time_now
            )
            for part in range(1, total_parts + 1)
        ]
        for future in futures:
            future.result()

    print(
        f"DONE  | Bulk vertex creation {length}^3={length*length*length} completed. |"
    )


def run_migration_segment(conn, part, total_parts, length, start_time):
    time_now = datetime.datetime.now()
    segments_per_axis = round(total_parts ** (1 / 3))
    segment_size = math.ceil(length / segments_per_axis)

    x_index = (part - 1) // (segments_per_axis**2)
    yz_part = (part - 1) % (segments_per_axis**2)
    y_index = yz_part // segments_per_axis
    z_index = yz_part % segments_per_axis

    x_start = x_index * segment_size
    y_start = y_index * segment_size
    z_start = z_index * segment_size

    x_end = min(x_start + segment_size, length)
    y_end = min(y_start + segment_size, length)
    z_end = min(z_start + segment_size, length)
    now = datetime.datetime.now()
    total_elapsed = now - start_time
    elapsed = now - time_now

    print(
        f"{total_elapsed} | {part}/{total_parts} | STARTED  | x {x_start}-{x_end} | y {y_start}-{y_end} | z {z_start}-{z_end} | {elapsed} | {elapsed} |"
    )

    vertex_creations = []

    for x in range(x_start, x_end):
        for y in range(y_start, y_end):
            for z in range(z_start, z_end):
                vertex_detail = f"""
                    (x{x}y{y}z{z}:Location {{
                        id: '{id()}',
                        name: '({x},{y},{z})',
                        vertex: map(["x", "y", "z"], [{{num:[{x}], str: []}}, {{num:[{y}], str: []}}, {{num:[{z}], str: []}}])
                    }})"""
                vertex_creations.append(vertex_detail)

    now = datetime.datetime.now()
    total_elapsed = now - start_time
    previous_elapsed = elapsed
    elapsed = now - time_now
    print(
        f"{total_elapsed} | {part}/{total_parts} | PREPARED | x {x_start}-{x_end} | y {y_start}-{y_end} | z {z_start}-{z_end} | {len(vertex_creations)} | {elapsed} | {elapsed - previous_elapsed} |"
    )

    combined_query = f"CREATE {', '.join(vertex_creations)}"

    execute(conn, combined_query, max_print_length=0)

    now = datetime.datetime.now()
    total_elapsed = now - start_time
    previous_elapsed = elapsed
    elapsed = now - time_now
    print(
        f"{total_elapsed} | {part}/{total_parts} | COMPLETE | x {x_start}-{x_end} | y {y_start}-{y_end} | z {z_start}-{z_end} | {len(vertex_creations)} | {elapsed} | {elapsed - previous_elapsed} |"
    )
