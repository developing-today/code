{
  lib,
  DNSConfig ? {
    # TODO: allow alias to work like cname for "@" and direct strings on domains
    "@" = {
      provider = "porkbun";
      records = {
        "@" = {
          "MX" = [
            {
              content = "monday.mxrouting.net";
              priority = 10;
            }
            {
              content = "monday-relay.mxrouting.net";
              priority = 20;
            }
          ];
          "TXT" = "v=spf1 include:mxlogin.com -all";
        };
      };
    };
    "1110x.de" = {
      records = {
        "x._domainkey" = {
          "TXT" =
            "v=DKIM1; k=rsa; p=MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAssK7hSU8x/oo2uXqsLEkGQ+rhKz/pViXrWOMrfQ/EMa/0r0ICIeIolkM3H3ZF/P60y1jAPdJ8Fq/G5ZvB5CAbP5k1mea6iq6q3SxNY0vMHOV6vLoha/65YIfAybn0vzHoDI44aSNeqZ16ku0EIv9wPiqGjSzYb+Zb5ZtnwtOe4JnmrPXjHgy4hYojVZd7E+bJqSHYKsAUqIT/1ZQiCQXGbqBISdqNGkW4TkWsOCKZhW1WwKdz/qaZF7S0jK1VEHJOb2c+B+Be+1OIvaQ1rOLWKpe6BV8b6FEV0kXnOsud9WzRg4bPU74QQclLrie7vaCb4wzp2JyIeHPKO32p16ZkwIDAQAB";
        };
      };
    };
    "1110x.com" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "0x1110.com" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "0x111.dev" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "0x1110.dev" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "1110x.dev" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "l0x.dev" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "grandnag.us" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "security.cab" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "default.computer" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "default.properties" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "rom.how" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "rom.run" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "developing.today" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "rom.rest" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "o3d.dev" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "printmy.pictures" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "79b.us" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "69b.us" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "64b.us" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "32b.us" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "15b.org" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "3dtropics.com" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "eau3d.org" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "developing-today.com" = {
      records = {
        #"@" = {
        #  "ALIAS" = "news.developing-today.com"; # alias needs a new resource type
        #};
        "news" = "dt.smol.pub";
        "zettel" = "developing-today.github.io";
        # "zulip" = {
        #   "ALIAS" = "developing-today.zulipchat.com"; # alias needs a new resource type
        # };
        # "git" = {
        #   "ALIAS" = "github.com/developing-today/code"; # alias needs a new resource type
        # };
        "archive.zulip" = "developing-today.github.io";
        "_github-pages-challenge-developing-today.zettel" = {
          "TXT" = "cc887eecc305202ad8c44465de9d1a";
        };
        "_github-pages-challenge-developing-today.archive.zulip" = {
          "TXT" = "cc748fb6c3246405abb50cbdc1fa9b";
        };
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "eau3d.dev" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "79b.org" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "64b.org" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "lair.cloud" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "s0s.pw" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "01b.us" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
        #
        "argo.internal" = {
          "A" = [ "10.10.32.1" ];
        };
        "argo" = {
          "A" = [ "10.10.32.1" ];
        };
        "control.internal" = {
          "A" = [
            "10.10.0.42"
            "10.10.8.188"
            "10.10.12.69"
            "10.10.24.137"
          ];
        };
        "control" = {
          "A" = [
            "10.10.0.42"
            "10.10.8.188"
            "10.10.12.69"
            "10.10.24.137"
          ];
        };
        "linstor.internal" = {
          "A" = [
            "10.10.6.85"
            "10.10.22.46"
            # "10.10.24.47"
            "10.10.17.129"
          ];
        };
        "linstor" = {
          "A" = [
            "10.10.6.85"
            "10.10.22.46"
            # "10.10.24.47"
            "10.10.17.129"
          ];
        };
        "linstor-controller.internal" = {
          "A" = [
            "10.10.6.85"
            "10.10.22.46"
            # "10.10.24.47"
            "10.10.17.129"
          ];
        };
        "linstor-controller" = {
          "A" = [
            "10.10.6.85"
            "10.10.22.46"
            # "10.10.24.47"
            "10.10.17.129"
          ];
        };
        "storage.internal" = {
          "A" = [
            "10.10.6.85"
            "10.10.22.46"
            # "10.10.24.47"
            "10.10.17.129"
          ];
        };
        "storage" = {
          "A" = [
            "10.10.6.85"
            "10.10.22.46"
            # "10.10.24.47"
            "10.10.17.129"
          ];
        };
      };
    };
    "15b.us" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "hax.live" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "developingto.day" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "yak.pub" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "printmy3dmodel.com" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "3dtropic.com" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "eau3d.com" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
    "carbonfreezeme.com" = {
      records = {
        # "x._domainkey" = {
        #   "TXT" = "TODO";
        # };
      };
    };
  },
  ...
}:
let
  safeToNullableInt =
    value:
    if value == null then
      null
    else if builtins.isInt value then
      value
    else if builtins.isString value then
      lib.toInt value
    else
      throw "Cannot convert ${builtins.typeOf value} to integer";
  generateRecords =
    domain: config:
    let
      replaceWithDomain =
        content: replaceValue:
        if replaceValue != null then
          builtins.replaceStrings [ replaceValue ] [ domain ] content
        else
          builtins.replaceStrings [ "@" ] [ domain ] content;
      processRecord =
        name: recordSet:
        let
          processEntry =
            type: content:
            let
              replaceValue = recordSet.replaceWithDomain or null;
              processContent =
                c:
                if builtins.isString c then
                  replaceWithDomain c replaceValue
                else if builtins.isAttrs c && c ? content then
                  c // { content = replaceWithDomain c.content replaceValue; }
                else
                  c;
            in
            if builtins.isString recordSet then
              [
                {
                  type = "CNAME";
                  content = processContent (if recordSet == "@" then domain else recordSet);
                }
              ]
            else if builtins.isString content then
              [
                {
                  inherit type;
                  content = processContent content;
                }
              ]
            else if builtins.isList content then
              map (
                item:
                if builtins.isString item then
                  {
                    inherit type;
                    content = processContent item;
                  }
                else
                  { inherit type; } // (processContent item)
              ) content
            else if builtins.isAttrs content then
              [ ({ inherit type; } // (processContent content)) ]
            else
              [
                {
                  inherit type;
                  content = processContent content;
                }
              ];

          flattenedRecords =
            if builtins.isString recordSet then
              processEntry "CNAME" recordSet
            else
              lib.flatten (
                lib.mapAttrsToList processEntry (builtins.removeAttrs recordSet [ "replaceWithDomain" ])
              );
        in
        map (
          r:
          r
          // {
            inherit domain name;
            type = lib.toUpper (
              r.type or (throw "type is required for domain: ${toString domain}, record: ${toString r}")
            );
          }
          // (lib.optionalAttrs (config.ttl or null != null) { ttl = safeToNullableInt config.ttl; })
          // (lib.optionalAttrs (config.priority or null != null) {
            priority = safeToNullableInt config.priority;
          })
        ) flattenedRecords;
      allRecords = lib.flatten (lib.mapAttrsToList processRecord (config.records or { }));
      defaultRecords =
        if domain != "@" then
          lib.flatten (lib.mapAttrsToList processRecord (DNSConfig."@".records or { }))
        else
          [ ];
    in
    allRecords ++ defaultRecords;

  allRecords = lib.flatten (lib.mapAttrsToList generateRecords (removeAttrs DNSConfig [ "@" ]));

  groupByProvider = lib.groupBy (
    r: DNSConfig.${r.domain}.provider or DNSConfig."@".provider or null
  ) allRecords;

  structureRecords =
    records:
    let
      deduplicateList =
        list:
        lib.unique (
          lib.sort (
            a: b:
            let
              strA = builtins.toJSON a;
              strB = builtins.toJSON b;
            in
            strA < strB
          ) list
        );

      addRecord =
        acc: r:
        let
          domainRecords = acc.${r.domain} or { };
          nameRecords = domainRecords.${r.name} or { };
          typeRecords = nameRecords.${r.type} or [ ];
          newRecord = removeAttrs r [
            "domain"
            "name"
            "type"
          ];
          updatedTypeRecords = deduplicateList (typeRecords ++ [ newRecord ]);
        in
        acc
        // {
          ${r.domain} = domainRecords // {
            ${r.name} = nameRecords // {
              ${r.type} = updatedTypeRecords;
            };
          };
        };
    in
    lib.foldl' addRecord { } records;

  finalStructure = lib.mapAttrs (provider: records: structureRecords records) groupByProvider;

  createCondensedRecords =
    records:
    let
      flattenRecord =
        domain: name: type: record: priorityIndex: itemIndex:
        let
          key = "${domain}_${name}_${type}_${toString priorityIndex}_${toString itemIndex}";
          ttl = safeToNullableInt (record.ttl or (DNSConfig."@".ttl or null));
          priority = safeToNullableInt (record.priority or (DNSConfig."@".priority or null));
          completeRecord =
            record
            // {
              inherit domain type;
            }
            // (if name != "@" then { inherit name; } else { })
            // (if ttl != null then { ttl = ttl; } else { })
            // (if priority != null then { priority = priority; } else { });
        in
        {
          ${key} = completeRecord;
        };

      flattenDomain =
        domain: domainRecords:
        lib.flatten (
          lib.mapAttrsToList (
            name: nameRecords:
            lib.flatten (
              lib.mapAttrsToList (
                type: typeRecords:
                let
                  sortedRecords = lib.sort (
                    a: b:
                    let
                      priorityA = a.priority or 0;
                      priorityB = b.priority or 0;
                      contentA = a.content or "";
                      contentB = b.content or "";
                    in
                    if priorityA != priorityB then priorityA < priorityB else contentA < contentB
                  ) typeRecords;
                  groupedByPriority = lib.groupBy (r: toString (r.priority or 0)) sortedRecords;
                  indexedRecords = lib.mapAttrsToList (
                    priority: group:
                    lib.imap0 (
                      itemIndex: record: flattenRecord domain name type record (safeToNullableInt priority) itemIndex
                    ) group
                  ) groupedByPriority;
                in
                lib.flatten indexedRecords
              ) nameRecords
            )
          ) domainRecords
        );

      flattenProvider =
        records:
        lib.flatten (
          lib.mapAttrsToList (domain: domainRecords: flattenDomain domain domainRecords) records
        );
    in
    lib.mapAttrs (
      provider: records:
      let
        flatRecords = flattenProvider records;
      in
      lib.foldl' (acc: record: acc // record) { } flatRecords
    ) records;

  condensedRecords = createCondensedRecords finalStructure;

  simplifyStructure =
    structure:
    lib.mapAttrs (
      provider: providerRecords:
      lib.mapAttrs (
        domain: domainRecords:
        lib.mapAttrs (
          name: nameRecords:
          let
            simplifiedNameRecords = lib.mapAttrs (
              type: typeRecords:
              if builtins.isList typeRecords && builtins.length typeRecords == 1 then
                let
                  record = builtins.head typeRecords;
                in
                if builtins.length (builtins.attrNames record) == 1 && builtins.hasAttr "content" record then
                  record.content
                else
                  record
              else
                typeRecords
            ) nameRecords;
          in
          if
            builtins.length (builtins.attrNames simplifiedNameRecords) == 1
            && builtins.hasAttr "CNAME" simplifiedNameRecords
          then
            simplifiedNameRecords.CNAME
          else
            simplifiedNameRecords
        ) domainRecords
      ) providerRecords
    ) structure;
in
{
  DNSConfig = simplifyStructure finalStructure // {
    "@" = condensedRecords;
  };
}
