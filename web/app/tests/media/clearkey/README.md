# ClearKey regression fixture

Three seconds of a generated 160×90, 10 fps FFmpeg test pattern. No footage,
addresses, licenses or keys from a provider are included. The project license
applies to this synthetic fixture.

The fragmented MP4 is encrypted with CENC AES-CTR using these public test values:

- KID: `00112233445566778899aabbccddeeff`
- Key: `000102030405060708090a0b0c0d0e0f`

Generation command, from this directory:

```sh
ffmpeg -f lavfi -i 'testsrc2=size=160x90:rate=10' -t 3 -an \
  -c:v libx264 -pix_fmt yuv420p -g 10 -keyint_min 10 -sc_threshold 0 \
  -f dash -seg_duration 1 -use_timeline 1 -use_template 1 \
  -format_options 'encryption_scheme=cenc-aes-ctr:encryption_key=000102030405060708090a0b0c0d0e0f:encryption_kid=00112233445566778899aabbccddeeff' \
  manifest.mpd
```

The MPD adds `urn:mpeg:dash:mp4protection:2011`, the default KID and a Widevine
ContentProtection/PSSH containing the same KID. It deliberately has no ClearKey
ContentProtection. `clearkey.spec.ts` supplies explicit playlist keys and checks
actual decoded frames and advancing media time with the bundled Shaka runtime.

The fixture needs no FFmpeg installation at test time. It reproduces the case
where an Android configuration explicitly selects ClearKey while the manifest
advertises other DRM systems. A simulated player cannot validate this behavior.
The same test switches to a Widevine session and inspects the real player's
configuration to ensure old ClearKey values and license systems were removed.

The binary fragments are stored as `.m4s.base64` text and decoded only by the
local test route. After generation, encode each fragment with Base64 and keep
the MPD as XML. No binary allowlist is added to the source audit.
