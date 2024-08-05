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
  safeToNullableInt = value:
    if value == null then null
    else if builtins.isInt value then value
    else if builtins.isString value then lib.toInt value
    else throw "Cannot convert ${builtins.typeOf value} to integer";
    generateRecords = domain: config:
      let
      replaceWithDomain = content: replaceValue:
        if replaceValue != null
        then builtins.replaceStrings [replaceValue] [domain] content
        else builtins.replaceStrings ["@"] [domain] content;
      processRecord = name: recordSet:
        let
          processEntry = type: content:
            let
              replaceValue = recordSet.replaceWithDomain or null;
              processContent = c:
                if builtins.isString c
                then replaceWithDomain c replaceValue
                else if builtins.isAttrs c && c ? content
                then c // { content = replaceWithDomain c.content replaceValue; }
                else c;
            in
            if builtins.isString recordSet then
              [{ type = "CNAME"; content = processContent (if recordSet == "@" then domain else recordSet); }]
            else if builtins.isString content then
              [{ inherit type; content = processContent content; }]
            else if builtins.isList content then
              map (item:
                if builtins.isString item then
                  { inherit type; content = processContent item; }
                else
                  { inherit type; } // (processContent item)
              ) content
            else if builtins.isAttrs content then
              [({ inherit type; } // (processContent content))]
            else
              [{ inherit type; content = processContent content; }];

              flattenedRecords = if builtins.isString recordSet
                                 then processEntry "CNAME" recordSet
                                 else lib.flatten (lib.mapAttrsToList processEntry (builtins.removeAttrs recordSet ["replaceWithDomain"]));
        in
        map (r: r // {
          inherit domain name;
          type = lib.toUpper (r.type or (throw "type is required for domain: ${toString domain}, record: ${toString r}"));
          } // (lib.optionalAttrs (config.ttl or null != null) { ttl = safeToNullableInt config.ttl; })
            // (lib.optionalAttrs (config.priority or null != null) { priority = safeToNullableInt config.priority; })
          ) flattenedRecords;
      allRecords = lib.flatten (lib.mapAttrsToList processRecord (config.records or {}));
      defaultRecords = if domain != "@" then lib.flatten (lib.mapAttrsToList processRecord (DNSConfig."@".records or {})) else [];
    in
    allRecords ++ defaultRecords;

  allRecords = lib.flatten (lib.mapAttrsToList generateRecords (removeAttrs DNSConfig ["@"]));

  groupByProvider = lib.groupBy (r: DNSConfig.${r.domain}.provider or DNSConfig."@".provider or null) allRecords;

  structureRecords = records:
    let
      deduplicateList = list:
        lib.unique (lib.sort (a: b:
          let
            strA = builtins.toJSON a;
            strB = builtins.toJSON b;
          in strA < strB
        ) list);

      addRecord = acc: r:
        let
          domainRecords = acc.${r.domain} or {};
          nameRecords = domainRecords.${r.name} or {};
          typeRecords = nameRecords.${r.type} or [];
          newRecord = removeAttrs r ["domain" "name" "type"];
          updatedTypeRecords = deduplicateList (typeRecords ++ [newRecord]);
        in
        acc // {
          ${r.domain} = domainRecords // {
            ${r.name} = nameRecords // {
              ${r.type} = updatedTypeRecords;
            };
          };
        };
    in
    lib.foldl' addRecord {} records;

  finalStructure = lib.mapAttrs (provider: records:
    structureRecords records
  ) groupByProvider;

  createCondensedRecords = records:
  let
    flattenRecord = domain: name: type: record: priorityIndex: itemIndex:
      let
        key = "${domain}_${name}_${type}_${toString priorityIndex}_${toString itemIndex}";
        ttl = safeToNullableInt (record.ttl or (DNSConfig."@".ttl or null));
        priority = safeToNullableInt (record.priority or (DNSConfig."@".priority or null));
        completeRecord = record // {
          inherit domain type;
        } // (if name != "@" then { inherit name; } else {})
        // (if ttl != null then { ttl = ttl; } else {})
        // (if priority != null then { priority = priority; } else {});
      in
      { ${key} = completeRecord; };

    flattenDomain = domain: domainRecords:
      lib.flatten (lib.mapAttrsToList (name: nameRecords:
        lib.flatten (lib.mapAttrsToList (type: typeRecords:
          let
            sortedRecords = lib.sort (a: b:
              let
                priorityA = a.priority or 0;
                priorityB = b.priority or 0;
                contentA = a.content or "";
                contentB = b.content or "";
              in
              if priorityA != priorityB
              then priorityA < priorityB
              else contentA < contentB
            ) typeRecords;
            groupedByPriority = lib.groupBy (r: toString (r.priority or 0)) sortedRecords;
            indexedRecords = lib.mapAttrsToList (priority: group:
              lib.imap0 (itemIndex: record:
                flattenRecord domain name type record (safeToNullableInt priority) itemIndex
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

  simplifyStructure = structure:
    lib.mapAttrs (provider: providerRecords:
      lib.mapAttrs (domain: domainRecords:
        lib.mapAttrs (name: nameRecords:
          let
            simplifiedNameRecords = lib.mapAttrs (type: typeRecords:
              if builtins.isList typeRecords && builtins.length typeRecords == 1
              then
                let record = builtins.head typeRecords;
                in if builtins.length (builtins.attrNames record) == 1 && builtins.hasAttr "content" record
                   then record.content
                   else record
              else typeRecords
            ) nameRecords;
          in
          if builtins.length (builtins.attrNames simplifiedNameRecords) == 1 && builtins.hasAttr "CNAME" simplifiedNameRecords
          then simplifiedNameRecords.CNAME
          else simplifiedNameRecords
        ) domainRecords
      ) providerRecords
    ) structure;
in
{
  DNSConfig = simplifyStructure finalStructure // { "@" = condensedRecords; };
}
