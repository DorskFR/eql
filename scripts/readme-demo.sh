#!/usr/bin/env bash
# Runs eqls against a throwaway Postgres and seeds one fake character for the README screenshots.
# Usage: scripts/readme-demo.sh up | down
set -euo pipefail

cd "$(dirname "$0")/.."
PG=eql-readme-pg
PG_PORT=5410
PORT=5411
TOKEN=demo-token
RUN=tmp/readme-demo
API="http://127.0.0.1:$PORT/api/v1"
CHARACTER=Ashvale
SERVER=erudin

down() {
    [[ -f $RUN/eqls.pid ]] && kill "$(cat "$RUN/eqls.pid")" 2>/dev/null || true
    docker rm -f "$PG" >/dev/null 2>&1 || true
    rm -rf "$RUN"
}

item() {
    local text
    text=$(printf '{{Itempage\n|itemname    = %s\n|statsblock  = \nMAGIC ITEM<br>\nSlot: %s<br>\n%s<br>\nWT: 1.0  Size: MEDIUM<br>\nClass: ALL<br>\nRace: ALL<br>\n}}\n' "$1" "$2" "$3")
    docker exec -i "$PG" psql -q -U eql -d eql -v name="$1" -v text="$text" \
        <<<"insert into items (name, stats, wikitext) values (:'name', '{}', :'text') on conflict do nothing;"
}

post() {
    curl -fsS -X "$1" -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
        --data-binary @- "$API/$2" >/dev/null
}

up() {
    down
    mkdir -p "$RUN"
    docker run -d --name "$PG" -e POSTGRES_USER=eql -e POSTGRES_PASSWORD=eql -e POSTGRES_DB=eql \
        -p "127.0.0.1:$PG_PORT:5432" pgautoupgrade/pgautoupgrade:18-alpine >/dev/null
    until docker exec "$PG" pg_isready -U eql -d eql >/dev/null 2>&1; do sleep 1; done

    cargo build -q -p eqls
    (cd web && npm run build >/dev/null)
    export DATABASE_URL="postgres://eql:eql@127.0.0.1:$PG_PORT/eql"
    EQLS_MACHINE_TOKEN=$TOKEN LISTEN_ADDR="127.0.0.1:$PORT" WEB_DIST=web/build \
        target/debug/eqls >"$RUN/eqls.log" 2>&1 &
    echo $! >"$RUN/eqls.pid"
    until curl -fsS "http://127.0.0.1:$PORT/readyz" >/dev/null 2>&1; do sleep 1; done
    until grep -q 'migrations applied' "$RUN/eqls.log"; do sleep 1; done

    for page in fixtures/wiki/*.wikitext; do
        name=$(basename "$page" .wikitext)
        docker exec -i "$PG" psql -q -U eql -d eql -v name="${name//_/ }" -v text="$(cat "$page")" \
            <<<"insert into items (name, stats, wikitext) values (:'name', '{}', :'text') on conflict do nothing;"
    done
    while IFS='|' read -r name slot stats; do
        item "$name" "$slot" "$stats"
    done <<'ITEMS'
Bronze Helm|HEAD|AC: 9<br>STA: +3  HP: +15
Iron Visor|FACE|AC: 4<br>STR: +2  WIS: +2
Silver Earring|EAR|AGI: +3  HP: +10<br>SV MAGIC: +5
Garnet Earring|EAR|DEX: +2  MANA: +10<br>SV COLD: +4
Bronze Collar|NECK|AC: 3<br>HP: +20<br>SV DISEASE: +6
Bloodstained Mantle|SHOULDERS|AC: 7<br>STR: +4  STA: +4
Bronze Gauntlets|HANDS|AC: 6<br>DEX: +3
Ringmail Leggings|LEGS|AC: 10<br>STA: +5  HP: +25
Bronze Boots|FEET|AC: 6<br>AGI: +2<br>SV POISON: +5
Thick Leather Belt|WAIST|AC: 3<br>STR: +3
Antiqued Silver Band|FINGERS|WIS: +3  INT: +3<br>SV MAGIC: +4
Onyx Ring|FINGERS|STR: +2  CHA: +4
Round Shield|SECONDARY|AC: 12<br>STA: +2
Throwing Knife|RANGE|DMG: 4  Atk Delay: 30
ITEMS
    target/debug/eqls reparse >/dev/null 2>&1

    local now
    now=$(date +%s)
    post POST events <<JSON
{"character":"$CHARACTER","server":"$SERVER","events":[
  {"at":$((now - 900)),"kind":"who","level":42,"classes":["SHD"],"race":"Erudite"},
  {"at":$((now - 600)),"kind":"zone","zone":"Stonebrunt Mountains"},
  {"at":$((now - 300)),"kind":"loot","item":"Spirit Reaver"},
  {"at":$((now - 120)),"kind":"level","level":42}
]}
JSON
    post POST inventory <<JSON
{"character":"$CHARACTER","server":"$SERVER","captured_at":$((now - 60)),"entries":[
  {"location":"Primary","name":"Spirit Reaver","id":0,"count":1,"slots":0},
  {"location":"Chest","name":"Rubicite Breastplate","id":0,"count":1,"slots":0},
  {"location":"Arms","name":"Azure Sleeves","id":0,"count":1,"slots":0},
  {"location":"Back","name":"Cloak of Flames","id":0,"count":1,"slots":0},
  {"location":"Wrist","name":"Azarack Skin Wristwraps","id":0,"count":1,"slots":0},
  {"location":"Head","name":"Bronze Helm","id":0,"count":1,"slots":0},
  {"location":"Face","name":"Iron Visor","id":0,"count":1,"slots":0},
  {"location":"Ear","name":"Silver Earring","id":0,"count":1,"slots":0},
  {"location":"Ear","name":"Garnet Earring","id":0,"count":1,"slots":0},
  {"location":"Neck","name":"Bronze Collar","id":0,"count":1,"slots":0},
  {"location":"Shoulders","name":"Bloodstained Mantle","id":0,"count":1,"slots":0},
  {"location":"Hands","name":"Bronze Gauntlets","id":0,"count":1,"slots":0},
  {"location":"Legs","name":"Ringmail Leggings","id":0,"count":1,"slots":0},
  {"location":"Feet","name":"Bronze Boots","id":0,"count":1,"slots":0},
  {"location":"Waist","name":"Thick Leather Belt","id":0,"count":1,"slots":0},
  {"location":"Fingers","name":"Antiqued Silver Band","id":0,"count":1,"slots":0},
  {"location":"Fingers","name":"Onyx Ring","id":0,"count":1,"slots":0},
  {"location":"Secondary","name":"Round Shield","id":0,"count":1,"slots":0},
  {"location":"Range","name":"Throwing Knife","id":0,"count":1,"slots":0},
  {"location":"General 1","name":"Bag of the Tinkerers","id":0,"count":1,"slots":10},
  {"location":"General 1-Slot1","name":"Bone Chips","id":0,"count":20,"slots":0}
]}
JSON
    for doc in fixtures/harvest/eql_*_Dorsk_erudin*.json; do
        kind=$(basename "$doc" | cut -d_ -f2)
        printf '{"character":"%s","server":"%s","kind":"%s","doc":%s}' \
            "$CHARACTER" "$SERVER" "$kind" "$(sed 's/Dorsk/Ashvale/g' "$doc")" | post POST harvest
    done
    printf '{"character":"%s","server":"%s","fights":%s}' \
        "$CHARACTER" "$SERVER" "$(cat fixtures/fights/eql_fights_Dorsk_erudin.json)" | post POST fights

    post POST layouts/demo/clone-default </dev/null

    echo "eqls is serving fake data on http://127.0.0.1:$PORT/c/$SERVER/$CHARACTER"
}

case "${1:-}" in
    up) up ;;
    down) down ;;
    *) echo "usage: $0 up|down" >&2; exit 2 ;;
esac
