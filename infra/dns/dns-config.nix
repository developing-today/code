# required
# type (String) The type of DNS Record to create
# domain (String) The base domain to to create the record on
# optional
# name (String) The subdomain for the record itself without the base domain
# content (String) The content of the record
# notes (String) Notes to add to the record
# prio (String) The priority of the record
# ttl (String) The ttl of the record, the minimum is 600
{
  lib,
  defaultDnsConfig ? {
    provider = "porkbun";
    ttl = 600;
    records = {
      "@" = {
        "MX" = [
          { content = "monday.mxroute.com"; priority = 10; }
          { content = "monday-relay.mxroute.com"; priority = 20; }
        ];
        "TXT" = "v=spf1 include:mxlogin.com -all"; # accepts string or list of strings or list of dicts/strings
      };
      "www" = "@"; # if string then it will be a CNAME record
      "mail" = "www.@"; # if string cname, assume @ is replaced with domain
      "blog" = "@"; # if string then it will be a CNAME record
    };
  },
  dnsConfig ? {
    "1110x.de" = {
      records = {
        "x._domainkey" = {
         "TXT" = {
           content = "v=DKIM1; k=rsa; p=MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAssK7hSU8x/oo2uXqsLEkGQ+rhKz/pViXrWOMrfQ/EMa/0r0ICIeIolkM3H3ZF/P60y1jAPdJ8Fq/G5ZvB5CAbP5k1mea6iq6q3SxNY0vMHOV6vLoha/65YIfAybn0vzHoDI44aSNeqZ16ku0EIv9wPiqGjSzYb+Zb5ZtnwtOe4JnmrPXjHgy4hYojVZd7E+bJqSHYKsAUqIT/1ZQiCQXGbqBISdqNGkW4TkWsOCKZhW1WwKdz/qaZF7S0jK1VEHJOb2c+B+Be+1OIvaQ1rOLWKpe6BV8b6FEV0kXnOsud9WzRg4bPU74QQclLrie7vaCb4wzp2JyIeHPKO32p16ZkwIDAQAB";
           ttl = 3600;
         };
        };
      };
    };
    "1110x.com" = { records = {"blog
    "0x1110.com" = {};
    "0x111.dev" = {};
    "0x1110.dev" = {};
    "1110x.dev" = {};
    "l0x.dev" = {};
    "grandnag.us" = {};
    "security.cab" = {};
    "default.computer" = {};
    "default.properties" = {};
    "rom.how" = {};
    "rom.run" = {};
    "developing.today" = {};
    "rom.rest" = {};
    "o3d.dev" = {};
    "printmy.pictures" = {};
    "79b.us" = {};
    "69b.us" = {};
    "64b.us" = {};
    "32b.us" = {};
    "15b.org" = {};
    "3dtropics.com" = {};
    "eau3d.org" = {};
    "developing-today.com" = {};
    "eau3d.dev" = {};
    "79b.org" = {};
    "64b.org" = {};
    "lair.cloud" = {};
    "s0s.pw" = {};
    "01b.us" = {};
    "15b.us" = {};
    "hax.live" = {};
    "developingto.day" = {};
    "yak.pub" = {};
    "printmy3dmodel.com" = {};
    "3dtropic.com" = {};
    "eau3d.com" = {};
    "carbonfreezeme.com" = {};
  },
  ...
}:
let
  generateRecords = domain: config:
    let
      defaultProvider = config.provider or defaultDnsConfig.provider;
      defaultTTL = config.ttl or defaultDnsConfig.ttl;
      processRecord = name: recordSet:
        let
          processEntry = type: content:
            if builtins.isString recordSet then
              [{ type = "CNAME"; content = if recordSet == "@" then domain else builtins.replaceStrings ["@"] [domain] recordSet; }]
            else if builtins.isString content then
              [{ inherit type content; }]
            else if builtins.isList content then
              map (item:
                if builtins.isString item then
                  { inherit type content; }
                else
                  { inherit type; } // (if item ? priority then item // { prio = item.priority; } else item)
              ) content
            else if builtins.isAttrs content then
              [({ inherit type; } // (if content ? priority then content // { prio = content.priority; } else content))]
            else
              [{ inherit type content; }];
          flattenedRecords = if builtins.isString recordSet
                             then processEntry "CNAME" recordSet
                             else lib.flatten (lib.mapAttrsToList processEntry recordSet);
        in
        map (r: r // {
          inherit domain;
          name = if name == "@" then "@" else name;
          provider = r.provider or defaultProvider;
          ttl = r.ttl or defaultTTL;
        } // (if r ? priority then { prio = r.priority; } else {})) flattenedRecords;
      allRecords = lib.flatten (lib.mapAttrsToList processRecord (config.records or {}));
      defaultRecords = lib.flatten (lib.mapAttrsToList processRecord defaultDnsConfig.records);
    in
    allRecords ++ defaultRecords;

  allRecords = lib.flatten (lib.mapAttrsToList generateRecords dnsConfig);

  groupByProvider = lib.groupBy (r: r.provider) allRecords;

  structureRecords = records:
    lib.foldl' (acc: r:
      let
        domainRecords = acc.${r.domain} or {};
        nameRecords = domainRecords.${r.name} or {};
        typeRecords = nameRecords.${r.type} or [];
        newRecord = removeAttrs r ["domain" "name" "type" "priority"];
      in
      acc // {
        ${r.domain} = domainRecords // {
          ${r.name} = nameRecords // {
            ${r.type} = typeRecords ++ [newRecord];
          };
        };
      }
    ) {} records;

  finalStructure = lib.mapAttrs (provider: records:
    structureRecords records
  ) groupByProvider;

  createCondensedRecords = records:
  let
        flattenRecord = domain: name: type: record: index:
          let
            key = "${domain}_${name}_${type}_${toString index}";
            completeRecord = removeAttrs record ["provider"] // {
              inherit domain type;
            } // (if name != "@" then { inherit name; } else {});
          in
          { ${key} = completeRecord; };

        flattenDomain = domain: domainRecords:
          lib.flatten (lib.mapAttrsToList (name: nameRecords:
            lib.flatten (lib.mapAttrsToList (type: typeRecords:
              lib.imap0 (index: record:
                flattenRecord domain name type record index
              ) typeRecords
            ) nameRecords)
          ) domainRecords);

        flattenProvider = records:
          lib.flatten (lib.mapAttrsToList (domain: domainRecords:
            flattenDomain domain domainRecords
          ) records);
      in
      lib.mapAttrs (provider: records:
        lib.foldl' (acc: record: acc // record) {} (flattenProvider records)
      ) records;
    condensedRecords = createCondensedRecords finalStructure;
in
{
  dnsConfig = finalStructure // { "@" = condensedRecords; };
}
