<script lang="ts">
  import Header from "./lib/Header.svelte";
  import Queue from "./pages/Queue.svelte";
  import About from "./pages/About.svelte";
  import Home from "./pages/Home.svelte";
  import NewPlate from "./pages/NewPlate.svelte";
  import PlateDetail from "./pages/PlateDetail.svelte";
  const path = window.location.pathname;
  const id = path.match(/^\/plates\/([0-9a-f-]+)$/)?.[1];
</script>

<svelte:head><title>OrcaServer</title></svelte:head>
<Header />
<main>
  {#if path === "/queue"}<Queue />
  {:else if path === "/about"}<About />
  {:else if path === "/plates/new"}<NewPlate />
  {:else if id}<PlateDetail {id} />
  {:else}<Home />{/if}
</main>
