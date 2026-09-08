# Maintainer: hemreduru <https://github.com/hemreduru>
pkgname=linotch
pkgver=0.1.0
pkgrel=1
pkgdesc="Usage notch for Linux: coding-assistant limits and media controls on a screen edge"
arch=('x86_64')
url="https://github.com/hemreduru/linotch"
license=('MIT')
depends=('gtk3' 'gtk-layer-shell')
makedepends=('rust' 'cargo')
source=("$pkgname-$pkgver.tar.gz::$url/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
  cd "$pkgname-$pkgver"
  cargo build --release --locked
}

check() {
  cd "$pkgname-$pkgver"
  cargo test --release --locked
}

package() {
  cd "$pkgname-$pkgver"
  install -Dm755 "target/release/$pkgname" "$pkgdir/usr/bin/$pkgname"
  install -Dm644 "$pkgname.desktop" "$pkgdir/usr/share/applications/$pkgname.desktop"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
