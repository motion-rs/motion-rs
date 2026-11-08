# Font Fixtures

The bundled Noto fonts are explicit test inputs, not system-font discovery or
runtime application defaults. They are distributed under SIL Open Font License
1.1; see the adjacent license files. No downloads are required to run the tests.

| File | Coverage used by tests | Upstream |
| --- | --- | --- |
| NotoSans-Regular.ttf | Latin, combining accents, ligatures | notofonts/noto-fonts, hinted/ttf/NotoSans |
| NotoSansHebrew-Regular.ttf | Hebrew bidirectional shaping | notofonts/noto-fonts, hinted/ttf/NotoSansHebrew |
| NotoSansCJKsc-Regular.otf | Simplified Chinese and fullwidth punctuation | notofonts/noto-cjk, Sans/OTF/SimplifiedChinese |

Pinned source revisions: noto-fonts ffebf8c1ee449e544955a7e813c54f9b73848eac;
noto-cjk f8d157532fbfaeda587e826d4cd5b21a49186f7c. Files are unmodified upstream
fonts (approximately 17 MiB total), not generated subsets.

SHA-256:

```text
b85c38ecea8a7cfb39c24e395a4007474fa5a4fc864f6ee33309eb4948d232d5  NotoSans-Regular.ttf
a7fa16fffb27bedb060a0866267c29e9859aeb9c21cc33f5b3aaf6eb062eca85  NotoSansHebrew-Regular.ttf
2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b  NotoSansCJKsc-Regular.otf
```

The text paragraph and line-breaking fixtures use these files by default.
MOTION_TEST_FONT and MOTION_TEST_FALLBACK_FONT can override the Latin and CJK inputs.
An override must retain the required coverage.

Other ignored text fixtures can use the same explicit paths. Color-emoji tests
additionally require MOTION_TEST_EMOJI_FONT with bitmap color glyphs for U+1F600
and the U+1F469 U+200D U+1F4BB sequence (for example Noto Color Emoji); a CJK or
monochrome font cannot substitute for that input.