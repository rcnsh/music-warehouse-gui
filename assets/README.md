# Icon

`icon.svg` is this app's member of the music-warehouse icon family: the
record stack with a magnifier badge. `icon-macos.svg` is the same art on
Apple's app-icon grid (an 824 px tile on a 1024 px canvas), and
`AppIcon.icns` is built from it. Render with a Chromium browser (ImageMagick
drops the badge), then let `sips` and `iconutil` do the rest:

```sh
printf '<body style="margin:0"><img src="icon-macos.svg" width="1024">' > /tmp/icon.html
cp icon-macos.svg /tmp/
chromium --headless=new --default-background-color=00000000 --window-size=1024,1024 \
  --screenshot=/tmp/icon-1024.png file:///tmp/icon.html
mkdir AppIcon.iconset
for s in 16 32 128 256 512; do
  sips -z $s $s /tmp/icon-1024.png --out AppIcon.iconset/icon_${s}x${s}.png
  sips -z $((s*2)) $((s*2)) /tmp/icon-1024.png --out AppIcon.iconset/icon_${s}x${s}@2x.png
done
iconutil -c icns AppIcon.iconset -o AppIcon.icns && rm -r AppIcon.iconset
```
