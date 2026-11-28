# kitty golden fixtures (#13623)

Test-owned, bounded inputs for the presenter-side stills pipeline golden
tests (`../kitty_golden.rs`). No guest or world bytes: both files are
generated once by a lane and checked in.

## Files

- `gradient-16x8.png` (120 bytes): 16x8 RGB gradient, red rising left to
  right (`R = x*255//15, G = 0, B = 128`). Generated with only the Python
  stdlib so no image tooling is needed to reproduce it:

  ```python
  import zlib, struct
  W, H = 16, 8
  rows = []
  for _ in range(H):
      px = bytearray()
      for x in range(W):
          px += bytes([x * 255 // (W - 1), 0, 128])
      rows.append(b"\x00" + bytes(px))
  raw = b"".join(rows)
  def chunk(tag, data):
      body = tag + data
      return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))
  png = (b"\x89PNG\r\n\x1a\n"
         + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
         + chunk(b"IDAT", zlib.compress(raw))
         + chunk(b"IEND", b""))
  open("gradient-16x8.png", "wb").write(png)
  ```

- `chafa-16x8-red-gradient.symbols`: headless reference rendering of that
  PNG with chafa 1.18.3 (via `nix run nixpkgs#chafa`):

  ```sh
  chafa -f symbols -s 16x8 --color-space rgb --dither none gradient-16x8.png \
    > chafa-16x8-red-gradient.symbols
  ```

  Four half-block rows of sixteen cells; foreground and background reds
  both rise left to right across the same dark-to-bright gradient the
  pipeline transmits. `kitty +kitten icat` was attempted as the second
  reference but refuses headless use (it needs a live terminal reporting
  pixel sizes: "Terminal does not support reporting screen sizes in
  pixels, use a terminal such as kitty, WezTerm, Konsole..."), so chafa
  is the checked-in reference and the icat attempt is recorded here
  instead of as a fixture.

## Regenerating

Re-run the two commands above; the golden test asserts the structural
shape (row count, monotonic gradient, adjacent-pixel pairing), not exact
bytes, so a newer chafa that renders the same gradient still passes.
