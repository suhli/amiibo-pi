<script lang="ts">
  import { onMount } from "svelte";
  import { links, scans, stamp, type Snapshot, type Tag } from "./model";
  const tabs = ["概览", "标签库", "手柄", "诊断", "设置"] as const;
  type Tab = (typeof tabs)[number];
  let page = $state<Tab>("概览"),
    token = $state(""),
    entered = $state(""),
    data = $state<Snapshot | null>(null);
  let online = $state(false),
    busy = $state(false),
    error = $state(""),
    search = $state(""),
    filter = $state(""),
    favorites = $state(false);
  let picked = $derived(data?.library.find((t) => t.selected)),
    ctrl = $derived(data?.controller),
    ready = $derived(online && ctrl?.link === "Ready");
  let armed = $derived(
    ctrl && ["Armed", "Discovered", "Reading"].includes(ctrl.scan),
  );
  let rows = $derived(
    (data?.library ?? []).filter(
      (t) =>
        (!favorites || t.favorite) &&
        (!filter || t.category === filter) &&
        [t.name, t.figure, t.uid, t.category]
          .join(" ")
          .toLowerCase()
          .includes(search.toLowerCase()),
    ),
  );
  let categories = $derived(
    Array.from(new Set(data?.library.map((t) => t.category).filter(Boolean))),
  );
  let ws: WebSocket | undefined,
    timer: ReturnType<typeof setTimeout> | undefined,
    alive = false,
    generation = 0,
    retry = 0;
  let categoryId = $state(0),
    categoryText = $state(""),
    uploadInput = $state<HTMLInputElement>();
  async function request(path: string, options: RequestInit = {}) {
    const r = await fetch("/api/" + path, {
      ...options,
      headers: { Authorization: "Bearer " + token, ...options.headers },
    });
    if (r.status === 401) {
      logout();
      throw Error("访问令牌无效");
    }
    const body = await r.json();
    if (!r.ok) throw Error(body.message ?? r.statusText);
    return body;
  }
  function disconnect() {
    generation++;
    clearTimeout(timer);
    ws?.close();
    ws = undefined;
    online = false;
  }
  function logout() {
    disconnect();
    sessionStorage.removeItem("amiibo-token");
    data = null;
    token = "";
    entered = "";
  }
  function retryLater(g: number) {
    if (!alive || g !== generation || !token) return;
    online = false;
    timer = setTimeout(
      () => connect(),
      Math.min(20000, 1000 * 2 ** Math.min(retry++, 5)),
    );
  }
  async function connect() {
    disconnect();
    const g = generation;
    try {
      const snapshot = (await request("snapshot")) as Snapshot;
      if (g !== generation || !alive) return;
      data = snapshot;
      sessionStorage.setItem("amiibo-token", token);
      ws = new WebSocket(
        (location.protocol === "https:" ? "wss://" : "ws://") +
          location.host +
          "/api/events",
        ["amiibo-v1", "auth." + token],
      );
      ws.onmessage = (e) => {
        if (g === generation) {
          data = JSON.parse(e.data);
          retry = 0;
          online = true;
        }
      };
      ws.onclose = () => retryLater(g);
    } catch (e) {
      if (alive) {
        error = String(e instanceof Error ? e.message : e);
        retryLater(g);
      }
    }
  }
  onMount(() => {
    alive = true;
    token = sessionStorage.getItem("amiibo-token") ?? "";
    if (token) connect();
    return () => {
      alive = false;
      disconnect();
    };
  });
  async function act(
    path: string,
    method = "POST",
    body?: BodyInit,
    headers?: HeadersInit,
  ) {
    if (busy || !online) return;
    busy = true;
    error = "";
    try {
      await request(path, { method, body, headers });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
  function editTag(t: Tag, category: string, favorite: boolean) {
    return act(
      "amiibo/" + t.id,
      "PATCH",
      JSON.stringify({ category, favorite }),
      { "Content-Type": "application/json" },
    );
  }
  function upload(file?: File) {
    if (!file) return;
    if (file.size > 572) {
      error = "备份超过 572 字节";
      return;
    }
    void act(
      "amiibo?name=" + encodeURIComponent(file.name.replace(/\.bin$/i, "")),
      "POST",
      file,
    );
  }
  const hex = (n: number | null | undefined) =>
    n == null ? "—" : "0x" + n.toString(16).padStart(2, "0");
</script>

<header>
  <div class="bar">
    <div class="brand">
      <span class="brand-icon" aria-hidden="true">◉</span> amiibo<span
        class="muted"
      >
        /
      </span>pi
    </div>
    {#if data}<span class:live={online} class="connection"
        ><i></i>{online ? "实时同步" : "正在恢复连接"}</span
      >{/if}
  </div>
</header>
{#if !data}
  <main class="login">
    <div class="panel">
      <h1>连接控制台</h1>
      <form
        onsubmit={(e) => {
          e.preventDefault();
          token = entered.trim();
          error = "";
          connect();
        }}
      >
        <label for="key">访问令牌</label><input
          id="key"
          type="password"
          bind:value={entered}
          autocomplete="current-password"
          required
        /><button class="primary">进入控制台</button>
      </form>
      {#if error}<p role="alert">{error}</p>{/if}
    </div>
  </main>
{:else}
  <nav aria-label="主导航">
    {#each tabs as t}<button
        class:current={page === t}
        onclick={() => (page = t)}>{t}</button
      >{/each}
  </nav>
  <main>
    <div class="title">
      <div>
        <p class="context">
          {data.system.adapter} <span> / </span>
          {ctrl?.simulation ? "Simulation" : "Bluetooth Classic"}
        </p>
        <h1>{page}</h1>
      </div>
      {#if page === "标签库"}<button
          class="primary"
          disabled={!online || busy}
          onclick={() => uploadInput?.click()}>＋ 导入备份</button
        >{/if}
    </div>
    <input
      hidden
      type="file"
      bind:this={uploadInput}
      onchange={(e) => {
        upload(e.currentTarget.files?.[0]);
        e.currentTarget.value = "";
      }}
    />
    {#if error}<div role="alert" class="alert">
        {error}<button onclick={() => (error = "")} aria-label="关闭提示"
          >×</button
        >
      </div>{/if}
    {#if ctrl?.error}<div class="alert">{ctrl.error}</div>{/if}
    {#if page === "概览" && ctrl}
      <div class="dashboard">
        <section class="panel scan-panel">
          <div class="section-head">
            <h2>扫描台</h2>
            <span class="chip">{scans[ctrl.scan]}</span>
          </div>
          <div class="tag-card">
            <div class="nfc-glyph" aria-hidden="true">
              <b></b><b></b><b></b>
            </div>
            <span class="minor">当前选择</span><strong
              >{picked?.name ?? "尚未选择标签"}</strong
            ><code>{picked?.uid ?? "—"}</code>
          </div>
          <label for="pick">amiibo 标签</label><select
            id="pick"
            value={picked?.id ?? ""}
            disabled={!online || busy || !!armed}
            onchange={(e) => act("amiibo/" + e.currentTarget.value + "/select")}
            ><option value="" disabled>选择一个备份</option
            >{#each data.library as t}<option value={t.id}>{t.name}</option
              >{/each}</select
          >
          {#if !data.library.length}<button onclick={() => (page = "标签库")}
              >打开标签库</button
            >{/if}
          <p class="hint">
            {armed
              ? "已放置 " +
                ctrl.armed_name +
                "。请在游戏中打开 amiibo 读取界面。"
              : ctrl.mtu !== null && ctrl.mtu < 363
                ? "当前通道可发送按键。NFC 读取需要至少 363 字节的 MTU。"
                : "准备标签后，由游戏发起读取请求。"}
          </p>
          <div class="actions">
            <button
              class="primary"
              disabled={!ready ||
                (ctrl.mtu ?? 0) < 363 ||
                !picked ||
                busy ||
                !!armed}
              onclick={() => act("nfc/arm")}>准备标签</button
            ><button
              disabled={!online || !armed || busy}
              onclick={() => act("nfc/cancel")}>移除标签</button
            >
            {#if ctrl.simulation}<button
                disabled={!online || ctrl.scan !== "Armed" || busy}
                onclick={() => act("simulation/replay")}>模拟游戏读取</button
              >{/if}
          </div>
        </section>
        <section class="panel">
          <h2>主机连接</h2>
          <strong class="state">{links[ctrl.link]}</strong>
          <dl>
            <dt>适配器</dt>
            <dd>{data.system.adapter}</dd>
            <dt>主机</dt>
            <dd>{ctrl.peer ?? "—"}</dd>
            <dt>Interrupt MTU</dt>
            <dd>{ctrl.mtu ?? "—"}</dd>
            <dt>报告模式</dt>
            <dd>{hex(ctrl.report)}</dd>
            <dt>MCU 状态</dt>
            <dd>{hex(ctrl.power)}</dd>
          </dl>
          <button onclick={() => (page = "手柄")}>手柄控制 →</button>
        </section>
      </div>
      <section class="panel">
        <h2>最近扫描</h2>
        {#if !data.history.length}<p class="empty">
            还没有扫描记录
          </p>{:else}{#each data.history.slice(0, 8) as h}<div class="entry">
              <div><strong>{h.name}</strong><small>{stamp(h.time)}</small></div>
              <span
                >{h.simulation ? "Simulation · " : ""}{scans[h.outcome]}</span
              >
            </div>{/each}{/if}
      </section>
    {:else if page === "标签库"}
      <section class="panel">
        <div class="filters">
          <input
            type="search"
            aria-label="搜索标签"
            placeholder="搜索名称、UID 或编号"
            bind:value={search}
          /><select aria-label="分类" bind:value={filter}
            ><option value="">全部分类</option>{#each categories as c}<option
                >{c}</option
              >{/each}</select
          ><button
            class:active={favorites}
            onclick={() => (favorites = !favorites)}>★ 收藏</button
          >
        </div>
        {#each rows as t (t.id)}<article class="entry tag">
            <div>
              <strong>{t.name}</strong>{#if t.selected}<span class="chip"
                  >当前选择</span
                >{/if}<small>UID {t.uid} · ID {t.figure}</small><small
                >{t.category || "未分类"}</small
              >
            </div>
            <div class="actions">
              <button
                aria-label={t.favorite ? "取消收藏" : "收藏"}
                disabled={!online || busy}
                onclick={() => editTag(t, t.category, !t.favorite)}
                >{t.favorite ? "★" : "☆"}</button
              ><button
                disabled={!online || busy || t.selected || !!armed}
                onclick={() => act("amiibo/" + t.id + "/select")}>选择</button
              ><button
                disabled={!online || busy}
                onclick={() => {
                  categoryId = t.id;
                  categoryText = t.category;
                }}>分类</button
              ><button
                disabled={!online || busy || !!armed}
                onclick={() => {
                  if (confirm("删除「" + t.name + "」的备份？"))
                    act("amiibo/" + t.id, "DELETE");
                }}>删除</button
              >
            </div>
          </article>
          {#if categoryId === t.id}<form
              class="actions"
              onsubmit={(e) => {
                e.preventDefault();
                editTag(t, categoryText, t.favorite);
                categoryId = 0;
              }}
            >
              <input
                aria-label="分类名称"
                bind:value={categoryText}
                maxlength="80"
              /><button>保存分类</button>
            </form>{/if}
        {:else}<p class="empty">
            {data.library.length
              ? "没有匹配的标签"
              : "导入自己合法获得的 NTAG215 备份（540 或 572 字节）。"}
          </p>{/each}
      </section>
    {:else if page === "手柄" && ctrl}
      <section class="panel">
        <div class="section-head">
          <h2>Pro Controller</h2>
          <span class="chip">{links[ctrl.link]}</span>
        </div>
        <dl>
          <dt>主机地址</dt>
          <dd>{ctrl.peer ?? "—"}</dd>
          <dt>Interrupt MTU</dt>
          <dd>{ctrl.mtu ?? "—"}</dd>
          <dt>报告模式</dt>
          <dd>{hex(ctrl.report)}</dd>
        </dl>
        <p class="hint">
          首次配对时，在 Switch 2 打开「手柄 → 更改握法／顺序」。
        </p>
        <div class="actions">
          <button
            class="primary"
            disabled={!online ||
              busy ||
              !["Idle", "Error", "Disconnected", "Reconnecting"].includes(
                ctrl.link,
              )}
            onclick={() => act("controller/pair")}>开始配对</button
          ><button
            disabled={!online ||
              busy ||
              !["Idle", "Error", "Disconnected", "Reconnecting"].includes(
                ctrl.link,
              )}
            onclick={() => act("controller/reconnect")}>重连</button
          ><button
            disabled={!online ||
              busy ||
              ["Idle", "Disconnected"].includes(ctrl.link)}
            onclick={() => act("controller/disconnect")}>断开</button
          >
        </div>
      </section>
      <section class="panel">
        <h2>按键测试</h2>
        <div class="actions">
          {#each ["A", "B", "X", "Y", "Plus", "L+R"] as key}<button
              disabled={!ready || busy}
              onclick={() =>
                act("controller/press?button=" + encodeURIComponent(key))}
              >{key}</button
            >{/each}
        </div>
      </section>
    {:else if page === "诊断"}
      <section class="panel">
        <h2>蓝牙诊断</h2>
        <pre>{data.system.management}</pre>
      </section>
      <section class="panel">
        <h2>事件日志</h2>
        {#each [...data.logs].reverse() as log}<div class="log">
            <time>{stamp(log.time)}</time><span>{log.message}</span>
          </div>{/each}
      </section>
    {:else if page === "设置"}
      <section class="panel">
        <h2>服务配置</h2>
        <dl>
          <dt>运行系统</dt>
          <dd>{data.system.os} · {data.system.arch}</dd>
          <dt>BlueZ</dt>
          <dd>{data.system.bluez}</dd>
          <dt>蓝牙服务</dt>
          <dd>{data.system.service}</dd>
          <dt>适配器</dt>
          <dd>{data.system.adapter}</dd>
          <dt>监听地址</dt>
          <dd>{data.bind}</dd>
          <dt>允许配对的主机</dt>
          <dd>{data.system.allowed_host || "配对窗口内的主机"}</dd>
          <dt>当前主机</dt>
          <dd>{ctrl?.peer ?? "—"}</dd>
        </dl>
        <p class="hint">
          在树莓派的 /etc/amiibo-pi 中配置监听地址、适配器和开机重连。
        </p>
      </section>
      <section class="panel">
        <h2>日志与访问</h2>
        <div class="actions">
          <button
            disabled={!online || busy}
            onclick={() => act("settings/debug?enabled=" + !data!.debug)}
            >{data.debug ? "关闭" : "开启"} HID 采样日志</button
          ><button onclick={() => logout()}>退出控制台</button>
        </div>
      </section>
    {/if}
  </main>{/if}
