/* =====================================================================
   Palette (spec: 256 indexed entries, RGB565, 0–15 named defaults)
   ===================================================================== */
const PAL_NAMES = ["BLACK","DARK_BLUE","PLUM","DARK_GREEN","BROWN","DARK_GREY","GREY","WHITE","RED","ORANGE","YELLOW","GREEN","BLUE","CYAN","MAGENTA","SAND"];
const PAL_DEFAULT = [0x1063,0x31AD,0x4126,0x3324,0x8266,0x4A49,0x9D36,0xEF9C,0xCA29,0xD3E5,0xDE8B,0x6D45,0x5BF9,0x6E19,0xA88E,0xBD4B];
const pal565 = new Uint16Array(256);
function resetPalette() { pal565.fill(0); PAL_DEFAULT.forEach((w, i) => pal565[i] = w); }
const exp5 = v => (v << 3) | (v >> 2), exp6 = v => (v << 2) | (v >> 4);
const rgbOf = w => [exp5(w >> 11 & 31), exp6(w >> 5 & 63), exp5(w & 31)];
resetPalette();
