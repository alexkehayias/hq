document.addEventListener('DOMContentLoaded', () => {
  const chartDiv = document.getElementById('chart');
  const loadingDiv = document.getElementById('loading');
  const emptyDiv = document.getElementById('empty');
  const errorDiv = document.getElementById('error');
  const timeRangeSelect = document.getElementById('timeRange');
  const metricTypeSelect = document.getElementById('metricType');
  const retryBtn = document.getElementById('retryBtn');

  // Summary metric elements
  const totalTokensEl = document.getElementById('totalTokens');
  const avgTokensPerDayEl = document.getElementById('avgTokensPerDay');
  const estCostEl = document.getElementById('estCost');
  const totalTokensLabelEl = document.getElementById('totalTokensLabel');
  const avgTokensPerDayLabelEl = document.getElementById(
    'avgTokensPerDayLabel',
  );
  const estCostCardEl = document.getElementById('estCostCard');

  // Per-bucket cost rates (USD per million tokens), based on Anthropic
  // Claude Opus 4.7 pricing. Cache reads are heavily discounted (replaying
  // cached context is cheap — 10% of input); cache writes carry a 1.25x
  // premium over fresh input (the provider charges more to populate the
  // cache, using the 5-minute write rate). Reasoning tokens are billed at
  // output rates (OpenAI o1-style).
  const COST_PER_MILLION = {
    input: 5.0,
    output: 25.0,
    cache_read: 0.5,
    cache_write: 6.25,
    reasoning: 25.0,
  };

  // Bucket labels + colors for the stacked chart (in display order).
  const BUCKETS = [
    { key: 'input', label: 'Input', color: '#3b82f6' },
    { key: 'cache_read', label: 'Cache Read', color: '#10b981' },
    { key: 'cache_write', label: 'Cache Write', color: '#f59e0b' },
    { key: 'output', label: 'Output', color: '#ef4444' },
    { key: 'reasoning', label: 'Reasoning', color: '#8b5cf6' },
  ];

  let chartInstance = null;

  function showState(state) {
    loadingDiv.classList.add('hidden');
    emptyDiv.classList.add('hidden');
    errorDiv.classList.add('hidden');

    if (state === 'loading') {
      loadingDiv.classList.remove('hidden');
      chartDiv.style.display = 'none';
    } else if (state === 'empty') {
      emptyDiv.classList.remove('hidden');
      chartDiv.style.display = 'none';
    } else if (state === 'error') {
      errorDiv.classList.remove('hidden');
      chartDiv.style.display = 'none';
    } else if (state === 'chart') {
      chartDiv.style.display = 'block';
    }
  }

  async function fetchMetrics() {
    showState('loading');

    const limitDays = timeRangeSelect.value;
    const metric = metricTypeSelect.value;
    setSummaryMode(metric);
    try {
      const response = await fetch(
        `/api/metrics?limit_days=${limitDays}&metric=${metric}`,
      );
      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }
      const data = await response.json();

      if (data.events && data.events.length > 0) {
        if (metric === 'sessions') {
          renderSessionsChart(data.events);
          updateSessionSummary(data.events);
        } else {
          renderChart(data.events);
          updateSummaryMetrics(data.events, parseInt(limitDays, 10));
        }
        showState('chart');
      } else {
        resetSummary();
        showState('empty');
      }
    } catch (error) {
      console.error('Error fetching metrics:', error);
      resetSummary();
      showState('error');
    }

    chartInstance?.resize();
  }

  function renderChart(events) {
    // Bucket each event by calendar day (events come pre-aggregated by
    // day from the API). UTC midnight keeps dates off-by-one clean.
    const dailyAggregates = new Map();
    for (const event of events) {
      const [year, month, day] = event.timestamp.split('-').map(Number);
      const jsIsStupidMonth = month - 1;
      const utcMidnight = new Date(year, jsIsStupidMonth, day).getTime();

      const existing = dailyAggregates.get(utcMidnight) ?? {
        input: 0,
        output: 0,
        cache_read: 0,
        cache_write: 0,
        reasoning: 0,
      };
      existing.input += event.input;
      existing.output += event.output;
      existing.cache_read += event.cache_read;
      existing.cache_write += event.cache_write;
      // reasoning is optional — legacy days may have null
      if (event.reasoning != null) {
        existing.reasoning += event.reasoning;
      }
      dailyAggregates.set(utcMidnight, existing);
    }

    const timeline = Array.from(dailyAggregates.keys()).sort();
    if (timeline.length === 0) {
      showState('empty');
      return;
    }

    const xAxisData = timeline.map((ts) => {
      const date = new Date(ts);
      return `${date.getMonth() + 1}/${date.getDate()}`;
    });

    // One stacked series per bucket so the chart shows daily totals
    // broken down by token category.
    const series = BUCKETS.map((bucket) => ({
      name: bucket.label,
      type: 'line',
      stack: 'tokens',
      areaStyle: { color: bucket.color },
      lineStyle: { width: 1, color: bucket.color },
      itemStyle: { color: bucket.color },
      smooth: true,
      connectNulls: false,
      data: timeline.map((ts) => {
        const agg = dailyAggregates.get(ts);
        return bucket.key === 'reasoning' && agg.reasoning === 0
          ? null // hide reasoning on days where it's absent
          : (agg[bucket.key] ?? 0);
      }),
    }));

    if (chartInstance) {
      chartInstance.dispose();
    }

    chartInstance = echarts.init(chartDiv);
    const option = {
      tooltip: {
        trigger: 'axis',
        axisPointer: { type: 'cross' },
      },
      legend: {
        data: BUCKETS.map((b) => b.label),
        bottom: 0,
      },
      grid: {
        left: '3%',
        right: '4%',
        bottom: '15%',
        containLabel: true,
      },
      xAxis: {
        type: 'category',
        boundaryGap: false,
        data: xAxisData,
      },
      yAxis: { type: 'value' },
      series,
    };

    chartInstance.setOption(option);

    window.addEventListener('resize', () => {
      if (chartInstance) {
        chartInstance.resize();
      }
    });
  }

  // Builds an x-axis label timeline shared by the charts.
  function buildTimeline(events) {
    const dailyAggregates = new Map();
    for (const event of events) {
      const [year, month, day] = event.timestamp.split('-').map(Number);
      const utcMidnight = new Date(year, month - 1, day).getTime();
      dailyAggregates.set(utcMidnight, event);
    }

    const timeline = Array.from(dailyAggregates.keys()).sort();
    const xAxisData = timeline.map((ts) => {
      const date = new Date(ts);
      return `${date.getMonth() + 1}/${date.getDate()}`;
    });
    return { timeline, xAxisData, dailyAggregates };
  }

  // Renders sessions per day as a bar chart.
  function renderSessionsChart(events) {
    const { timeline, xAxisData, dailyAggregates } = buildTimeline(events);

    const data = timeline.map((ts) => {
      const event = dailyAggregates.get(ts);
      return event.value ?? 0;
    });

    if (chartInstance) {
      chartInstance.dispose();
    }

    chartInstance = echarts.init(chartDiv);
    const option = {
      tooltip: {
        trigger: 'axis',
        axisPointer: { type: 'shadow' },
      },
      grid: {
        left: '3%',
        right: '4%',
        bottom: '15%',
        containLabel: true,
      },
      xAxis: {
        type: 'category',
        boundaryGap: true,
        data: xAxisData,
      },
      yAxis: { type: 'value', minInterval: 1 },
      series: [
        {
          name: 'Sessions',
          type: 'bar',
          data,
          itemStyle: { color: '#3b82f6' },
          smooth: false,
        },
      ],
    };

    chartInstance.setOption(option);

    window.addEventListener('resize', () => {
      if (chartInstance) {
        chartInstance.resize();
      }
    });
  }

  function totalTokensFor(event) {
    return (
      event.input +
      event.output +
      event.cache_read +
      event.cache_write +
      (event.reasoning ?? 0)
    );
  }

  function costFor(event) {
    return (
      (event.input * COST_PER_MILLION.input +
        event.output * COST_PER_MILLION.output +
        event.cache_read * COST_PER_MILLION.cache_read +
        event.cache_write * COST_PER_MILLION.cache_write +
        (event.reasoning ?? 0) * COST_PER_MILLION.reasoning) /
      1_000_000
    );
  }

  function updateSummaryMetrics(events, _limitDays) {
    if (events.length === 0) {
      resetSummary();
      return;
    }

    const totalTokens = events.reduce((sum, e) => sum + totalTokensFor(e), 0);
    const estCost = events.reduce((sum, e) => sum + costFor(e), 0);

    // Average per day with data (not per day in the window — days without
    // events don't count against the average).
    const uniqueDays = new Set(events.map((e) => e.timestamp)).size;
    const avgTokensPerDay = uniqueDays > 0 ? totalTokens / uniqueDays : 0;

    totalTokensEl.textContent = formatNumber(totalTokens);
    avgTokensPerDayEl.textContent = formatNumber(Math.round(avgTokensPerDay));
    estCostEl.textContent = `$${estCost.toFixed(2)}`;
  }

  function updateSessionSummary(events) {
    if (events.length === 0) {
      resetSummary();
      return;
    }

    const totalSessions = events.reduce((sum, e) => sum + (e.value ?? 0), 0);
    const uniqueDays = new Set(events.map((e) => e.timestamp)).size;
    const avgSessionsPerDay = uniqueDays > 0 ? totalSessions / uniqueDays : 0;

    totalTokensEl.textContent = formatNumber(totalSessions);
    avgTokensPerDayEl.textContent = formatNumber(Math.round(avgSessionsPerDay));
    estCostEl.textContent = '--';
  }

  function resetSummary() {
    totalTokensEl.textContent = '--';
    avgTokensPerDayEl.textContent = '--';
    estCostEl.textContent = '--';
  }

  // Swaps the summary card labels for the selected metric. Cost only makes
  // sense for tokens, so it's hidden in sessions mode.
  function setSummaryMode(metric) {
    if (metric === 'sessions') {
      totalTokensLabelEl.textContent = 'Total Sessions';
      avgTokensPerDayLabelEl.textContent = 'Avg Sessions/Day';
      estCostCardEl.classList.add('hidden');
    } else {
      totalTokensLabelEl.textContent = 'Total Tokens';
      avgTokensPerDayLabelEl.textContent = 'Avg Tokens/Day';
      estCostCardEl.classList.remove('hidden');
    }
  }

  function formatNumber(num) {
    if (num >= 1_000_000) {
      return `${(num / 1_000_000).toFixed(1)}M`;
    } else if (num >= 1_000) {
      return `${(num / 1_000).toFixed(1)}K`;
    }
    return num.toString();
  }

  // Event listeners
  timeRangeSelect.addEventListener('change', fetchMetrics);
  metricTypeSelect.addEventListener('change', fetchMetrics);
  retryBtn.addEventListener('click', fetchMetrics);

  // Initial load
  fetchMetrics();
});
