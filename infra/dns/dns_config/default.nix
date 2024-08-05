{
  lib,
  DNSConfig ? {
    "@" = {
      provider = "porkbun";
      records = {
        "@" = {
          "MX" = [
            { content = "monday.mxroute.com"; priority = 10; }
            { content = "monday-relay.mxroute.com"; priority = 20; }
          ];
          "TXT" = "v=spf1 include:mxlogin.com -all";
        };
        "www" = "@";
        "mail" = "www.@";
        "blog" = "@";
      };
    };
    "1110x.de" = {
      provider = "banana"; # TODO: this should work
      ttl = "4200"; # TODO: this should work
      priority = "42"; # TODO: this should work
      records = {
        "x._domainkey" = {
         "TXT" = {
           content = "v=DKIM1; k=rsa; p=MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAssK7hSU8x/oo2uXqsLEkGQ+rhKz/pViXrWOMrfQ/EMa/0r0ICIeIolkM3H3ZF/P60y1jAPdJ8Fq/G5ZvB5CAbP5k1mea6iq6q3SxNY0vMHOV6vLoha/65YIfAybn0vzHoDI44aSNeqZ16ku0EIv9wPiqGjSzYb+Zb5ZtnwtOe4JnmrPXjHgy4hYojVZd7E+bJqSHYKsAUqIT/1ZQiCQXGbqBISdqNGkW4TkWsOCKZhW1WwKdz/qaZF7S0jK1VEHJOb2c+B+Be+1OIvaQ1rOLWKpe6BV8b6FEV0kXnOsud9WzRg4bPU74QQclLrie7vaCb4wzp2JyIeHPKO32p16ZkwIDAQAB";
           ttl = 3600;
         };
        };
      };
    };
    "1110x.com" = {};
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
                  { inherit type; } // item
              ) content
            else if builtins.isAttrs content then
              [({ inherit type; } // content)]
            else
              [{ inherit type content; }];
          flattenedRecords = if builtins.isString recordSet
                             then processEntry "CNAME" recordSet
                             else lib.flatten (lib.mapAttrsToList processEntry recordSet);
        in
        map (r: r // {
          inherit domain name;
          type = lib.toUpper (r.type or (throw "type is required for domain: ${toString domain}, record: ${toString r}"));
          } // (lib.optionalAttrs (config.provider or null != null) { inherit (config) provider; })
                    // (lib.optionalAttrs (config.ttl or null != null) { inherit (config) ttl; })
                    // (lib.optionalAttrs (config.priority or null != null) { inherit (config) priority; })
                  ) flattenedRecords;
      allRecords = lib.flatten (lib.mapAttrsToList processRecord (config.records or {}));
      defaultRecords = if domain != "@" then lib.flatten (lib.mapAttrsToList processRecord (DNSConfig."@".records or {})) else [];
    in
    allRecords ++ defaultRecords;

  allRecords = lib.flatten (lib.mapAttrsToList generateRecords (removeAttrs DNSConfig ["@"]));

  groupByProvider = lib.groupBy (r: r.provider or (DNSConfig."@".provider or null)) allRecords;

  structureRecords = records:
    lib.foldl' (acc: r:
      let
        domainRecords = acc.${r.domain} or {};
        nameRecords = domainRecords.${r.name} or {};
        typeRecords = nameRecords.${r.type} or [];
        newRecord = removeAttrs r ["domain" "name" "type"];
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
    flattenRecord = domain: name: type: record: priorityIndex: itemIndex:
      let
        key = "${domain}_${name}_${type}_${toString priorityIndex}_${toString itemIndex}";
        provider = record.provider or (DNSConfig."@".provider or (throw "No provider specified for ${domain}"));
        ttl = record.ttl or (DNSConfig."@".ttl or 600); # TODO: just allow empty and rely on per-provider defaults or hardcode defaults in terraform code?
        priority = record.priority or (DNSConfig."@".priority or 0); # TODO: just allow empty and rely on per-provider defaults or hardcode defaults in terraform code?
        completeRecord = removeAttrs record ["provider"] // {
          inherit domain type provider ttl priority;
        } // (if name != "@" then { inherit name; } else {});
      in
      { ${key} = completeRecord; };

    flattenDomain = domain: domainRecords:
      lib.flatten (lib.mapAttrsToList (name: nameRecords:
        lib.flatten (lib.mapAttrsToList (type: typeRecords:
          let
            sortedRecords = lib.sort (a: b: (a.priority or 0) < (b.priority or 0)) typeRecords;
            groupedByPriority = lib.groupBy (r: toString (r.priority or 0)) sortedRecords;
            indexedRecords = lib.mapAttrsToList (priority: group:
              lib.imap0 (itemIndex: record:
                flattenRecord domain name type record (lib.toInt priority) itemIndex
              ) group
            ) groupedByPriority;
          in
          lib.flatten indexedRecords
        ) nameRecords)
      ) domainRecords);

    flattenProvider = records:
      lib.flatten (lib.mapAttrsToList (domain: domainRecords:
        flattenDomain domain domainRecords
      ) records);
  in
  lib.mapAttrs (provider: records:
    let
      flatRecords = flattenProvider records;
    in
    lib.foldl' (acc: record: acc // record) {} flatRecords
  ) records;

  condensedRecords = createCondensedRecords finalStructure;
in
{
  DNSConfig = finalStructure // { "@" = condensedRecords; };
}
