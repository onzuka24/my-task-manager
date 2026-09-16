<script lang="ts">
  // オーバーレイは通常ウィンドウとして実装し、フォーカスを取得してよい (AD-6)。
  // 介入 (CAP-10) は非活性パネルとして別に実装するため、ここでは兼ねない。
  import Overlay from './overlay/Overlay.svelte'
</script>

<Overlay />
