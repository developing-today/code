import kuzu

db = kuzu.Database("./data")
conn = kuzu.Connection(db)

# Define the schema
# conn.execute("CREATE NODE TABLE Version(name STRING, counter INT64, PRIMARY KEY (counter))")
# try getnewest version else create new version
try:
    results = conn.execute(
        "MATCH (v:Version) RETURN v.name, v.counter ORDER BY v.counter DESC LIMIT 1;"
    )
    while results.has_next():
        print(results.get_next())
except:
    conn.execute(
        "CREATE NODE TABLE Version(name STRING, counter INT64, PRIMARY KEY (counter))"
    )
    conn.execute('CREATE (v:Version {name: "v1", counter: 1});')
    print("Version table created")
    results = conn.execute(
        "MATCH (v:Version) RETURN v.name, v.counter ORDER BY v.counter DESC LIMIT 1;"
    )
    while results.has_next():
        print(results.get_next())

# conn.execute("CREATE NODE TABLE User(name STRING, age INT64, PRIMARY KEY (name))")
# conn.execute("CREATE NODE TABLE City(name STRING, population INT64, PRIMARY KEY (name))")
# conn.execute("CREATE REL TABLE Follows(FROM User TO User, since INT64)")
# conn.execute("CREATE REL TABLE LivesIn(FROM User TO City)")

# Load some data
# conn.execute('COPY User FROM "user.csv"')
# conn.execute('COPY City FROM "city.csv"')
# conn.execute('COPY Follows FROM "follows.csv"')
# conn.execute('COPY LivesIn FROM "lives-in.csv"')

# Query the data
# results = conn.execute('MATCH (u:User) RETURN u.name, u.age;')
# while results.has_next():
#     print(results.get_next())
