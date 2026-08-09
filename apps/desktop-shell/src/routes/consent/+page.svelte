<script lang="ts">
  /// The consent surface's page. Deliberately empty.
  ///
  /// The card itself is mounted by the root layout, gated on this window's label,
  /// because that gate is where the one-request-one-surface rule already lives: an
  /// earlier boot had two windows each polling the broker and rendering their own
  /// copy, and the same request got answered twice. Keeping the mount in one place
  /// keeps that rule in one place.
  ///
  /// The window exists at all because keyboard focus on a layer surface is decided
  /// when it maps. The card used to live in the top bar, which maps once at startup
  /// with no keyboard interactivity, and flipping that property later changed
  /// nothing anyone read again - so Escape-to-deny never fired. This window maps
  /// exclusive, the way the waypointer does.
</script>
