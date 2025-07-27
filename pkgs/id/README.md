```

# TypeCharacteristics
# TypeDetails
Type
CreateType

# TypeLinkCharacteristics
# TypeLinkDetails
# TypeLinkType
TypeLink
CreateTypeLink

# NamespaceCharacteristics
# NamespaceDetails
# NamespaceType
Namespace
CreateNamespace

# url/uri/urn whatever
# url = namespace://(path(?(key=(value(,).)+)+).).
# named/custom ids
# custom-namespaces for specific content types or user-provided/managed
# hash-based ids different formats including custom treed hash of structured data kdl/json/xml/automerge (each part breaks out into it's own entity)
# resource-based/registry? specific format rules/etc.
# random-based id cuid2 / uuidv1/etc.
# time-based id cuid1 / uuidv6/7/8? / etc. with/without partitioning/subsecond
# versioned/semver/git/scm/nix/flakes/docker/npm/pip/nuget/go/etc.

# IdCharacteristics
# IdDetails
# IdType
Id
CreateId

# Hash

#FormatCharacteristics
#FormatDetails
#FormatType
Format
CreateFormat

#ContentCharacteristics
#ContentDetails
#ContentType
Content
CreateContent

# RawContent 
# content has a url to data,
# maybe a hash / format
# for data platform has interned it could be in rawcontent
# this could be a cached replacement or the primary source
# which may be a sql table, redis cache,
#   large files on local disk
#   s3 path
#   websocket call/response or logs
#   hashmap / in-ram cache
#   

# Entity

#LinkCharacteristics
#LinkDetails
#LinkType?
Link
CreateLink

#LicenseCharacteristics?
#LicenseDetails?
#LicenseType?
License
CreateLicense
#LicenseCharacteristics?

#RoomCharacteristics?
#RoomDetails?
#RoomType?
Room : { name : string, location : list(string)|string }
GetRoom(string id) : Room
GetDefaultRoom() : GetRoom("")
GetLocation(list(string)|string) : Location
CreateRoom


# Something to control privileges
# globally at the platform level
# per-entity/content
# for now:
#   read: all is public, maybe 2 levels admin/read, secrets stored in admin and admin can see them
#   write: only admins or the owner of named id can 'edit'. hashed things can't be edited. a new object can be made. delete and replaces links can be added.


# Something to control tokens/utilization
# tigerbeetle account controlled by owner of given id, 'fiat://id_url', infinite credits of fiat:id_url for owner, ability to give credits to other ids, no ability to revoke credits or it's separate privilege and impossible for some accounts.. once an account is made for an id that user can always trade/receive in that token. empty accounts may be deleted by user-owners..
# deletion is a marker on the account, nothing happens to any data.



# querying
# ways to return more than one result, query a specific attribute or id that may have more than one definition
# query down a tree following links of type X (callback function to allow any kind of traversal)



# retention policies
# indexed vs kept
# ttl based on access/edit
# ram/hashmap -> ring buffer -> local file optane/nvme -> redis -> websocket -> optane db (sqlite/duck/postgres) -> nvme db (sqlite/duck/postgres) -> optane file -> nvme file -> nvme seaweed -> hdd seaweed
# delete tag -> prune/compress -> pre-emptive index/cache
# lru cache of pull/used // on-startup file // Ids_Seen // streamed/published by controller









```
