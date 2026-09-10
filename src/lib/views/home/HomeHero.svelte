<script lang="ts">
  import { isAndroid } from '../../platform';

  export let hk1: string;
  export let hk2: string;
  export let android = false;

  // Svelte's legacy prop bridge can briefly supply the default value while
  // the Android WebView is bootstrapping. Read the platform directly as well
  // so the primary Android instruction never falls back to desktop copy.
  $: showAndroid = android || isAndroid;
</script>

<div class="hero-photo">
  <div class="hero-photo-content">
    <h2 class="hero-photo-title">
      {#if showAndroid}
        Tap the Verenu pill to dictate
      {:else}
        Hold <kbd>{hk1}</kbd> <kbd>{hk2}</kbd> to dictate
      {/if}
    </h2>
    <p class="hero-photo-sub">
      {#if showAndroid}
        Open a text field and use the pill above your keyboard.
      {:else}
        Verenu works in any app. Try it in
        <em class="hero-em">email, messages, docs</em> &mdash; or anywhere else.
      {/if}
    </p>
  </div>
</div>

<style>
  .hero-photo {
    position: relative;
    border-radius: var(--r-lg);
    overflow: hidden;
    margin-bottom: 16px;
    height: clamp(104px, 12vw, 136px);
    background: var(--arm-950);
  }

  /*
   * Covers the whole card and positions the gradient's centre, rather than
   * being a smaller offset box: a box narrower than its own falloff clips the
   * glow into a hard-edged rectangle down one side.
   */
  .hero-photo::before {
    content: '';
    position: absolute;
    inset: 0;
    background: radial-gradient(
      120% 180% at 88% -20%,
      color-mix(in srgb, var(--hero-accent) 22%, transparent) 0%,
      transparent 62%
    );
    pointer-events: none;
  }

  .hero-photo-content { position: relative; padding: 18px 24px; max-width: min(500px, 100%); }

  .hero-photo-title {
    font-family: var(--sans);
    font-size: 19px;
    font-weight: 500;
    letter-spacing: -0.02em;
    margin: 0 0 8px;
    line-height: 1.15;
    color: #ffffff;
  }

  .hero-photo-title :global(kbd) {
    background: rgb(255 255 255 / 10%);
    border: 1px solid rgb(255 255 255 / 18%);
    border-radius: 5px;
    font-family: var(--mono);
    font-size: 13px;
    padding: 1px 6px;
    color: var(--hero-accent);
    font-weight: 500;
  }

  .hero-photo-sub {
    font-size: 12.5px;
    color: rgb(255 255 255 / 70%);
    margin: 0;
    line-height: 1.5;
  }

  .hero-em {
    font-family: var(--sans);
    font-style: italic;
    color: rgb(255 255 255 / 90%);
  }

  @media (max-width: 720px) {
    /*
     * Phones size the card to its text instead of pinning it to a fixed
     * height: at these widths the subtitle wraps to two or three lines and the
     * 104px floor clipped it. The glow is scaled to the card so it reads as a
     * soft corner wash rather than a hard-edged block sitting mid-card.
     */
    .hero-photo {
      height: auto;
      min-height: 96px;
    }

    .hero-photo-content { padding: 16px 18px; max-width: 100%; }
    .hero-photo-title { font-size: 17px; margin-bottom: 6px; }
    .hero-photo-sub { font-size: 12px; }
  }
</style>
