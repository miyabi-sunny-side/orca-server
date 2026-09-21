<script lang="ts">
  import Header from "./lib/Header.svelte";
  import Filaments from "./pages/Filaments.svelte";
  import Ams from "./pages/Ams.svelte";
  import Printers from "./pages/Printers.svelte";
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
  {#if /^\/printers\/[^/]+\/ams$/.test(path)}<Ams />
  {:else if path === "/filaments" || path.startsWith("/filaments/")}<Filaments
    />
  {:else if path === "/printers" || path.startsWith("/printers/")}<Printers />
  {:else if path === "/" || path === "/queue"}<Queue />
  {:else if path === "/about"}<About />
  {:else if path === "/plates/new"}<NewPlate />
  {:else if id}<PlateDetail {id} />
  {:else}<Home />{/if}
</main>
