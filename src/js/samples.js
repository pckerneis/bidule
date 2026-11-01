/* =====================================================================
   Sample carts
   ===================================================================== */
const SRC_SPARK = SAMPLE_SOURCES["spark"];

const SRC_BLINK = SAMPLE_SOURCES["blink"];

const SRC_PEEK = SAMPLE_SOURCES["peek"];

const SRC_EDITOR = SAMPLE_SOURCES["sprites"];

// FNV-1a over the source text: lets the shelf update untouched sample carts
function srcHash(s) { let h = 0x811c9dc5; for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 0x01000193) >>> 0; } return h; }
// sources of earlier sample releases, by hash, so unedited copies get the new version
const OLD_SAMPLE_HASHES = { sprites: [3151107878] };

function makeSampleCart(id) {
  const sec = emptySections();
  if (id === "spark") {
    paintTiles(sec.sprites, {
      1: ["00DDDD00", "0DDDDDD0", "D71DD71D", "D11DD11D", "DDDDDDDD", "DDCDDCDD", "0DDCCDD0", "00DDDD00"],
      2: ["00000000", "00090000", "000A0000", "09A7A900", "000A0000", "00090000", "00000000", "00000000"],
      3: ["00000000", "0A000A00", "00909000", "00070000", "00909000", "0A000A00", "00000000", "00000000"],
      4: ["00002000", "00000000", "00000000", "00000000", "00000000", "00000000", "00000000", "00000000"],
    });
    sec.sprites.flags[1] = 1;
    sec.sprites.slots = [
      { name: "PLAYER", tile: 1, w: 1, h: 1 },
      { name: "SPARK_A", tile: 2, w: 1, h: 1 },
      { name: "SPARK_B", tile: 3, w: 1, h: 1 },
    ];
    sec.maps.slots = [{ name: "BACKDROP", w: 30, h: 18, cells: new Uint16Array(30 * 18).fill(4) }];
    sec.sounds.slots = [
      { name: "CATCH", bytes: soundBytes({ wave: 1, vol: 180, notes: [88, 95], step: 50 }) },
      { name: "DASH", bytes: soundBytes({ wave: 3, vol: 100, from: 900, to: 200, time: 80 }) },
      { name: "OVER", bytes: soundBytes({ wave: 0, vol: 130, duty: 64, notes: [67, 64, 60], step: 120 }) },
    ];
    return { id: "sample-spark", sampleId: "spark", source: SRC_SPARK, sections: sec };
  }
  if (id === "blink") return { id: "sample-blink", sampleId: "blink", source: SRC_BLINK, sections: sec };
  if (id === "sprites") {
    paintTiles(sec.sprites, { 1: ["00000000", "0000AA00", "000AA900", "00AA9000", "0AA90000", "0C900000", "0C000000", "00000000"] });
    sec.sprites.slots = [{ name: "ICON", tile: 1, w: 1, h: 1 }];
    return { id: "sample-sprites", sampleId: "sprites", source: SRC_EDITOR, sections: sec };
  }
  if (id === "peek") {
    paintTiles(sec.sprites, { 1: ["77777777", "70000007", "70A00A07", "70000007", "70A00A07", "700AA007", "70000007", "77777777"] });
    sec.sprites.slots = [{ name: "ICON", tile: 1, w: 1, h: 1 }];
    return { id: "sample-peek", sampleId: "peek", source: SRC_PEEK, sections: sec };
  }
  return null;
}
const SAMPLE_IDS = ["sprites", "spark", "peek", "blink"];
{ const mk = makeSampleCart; makeSampleCart = id => { const c = mk(id); if (c) c.pristine = srcHash(c.source); return c; }; }
const NEW_CART_SRC = SAMPLE_SOURCES["new-cart"];
