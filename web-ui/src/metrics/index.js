document.addEventListener('DOMContentLoaded', () => {
  const chartDiv = document.getElementById('chart');
  const loadingDiv = document.getElementById('loading');
  const emptyDiv = document.getElementById('empty');
  const errorDiv = document.getElementById('error');
  const timeRangeSelect = document.getElementById('timeRange');
  const retryBtn = document.getElementById('retryBtn');

  // Summary metric elements
  const totalTokensEl = document.getElementById('totalTokens');
  const avgTokensPerDayEl = document.getElementById('avgTokensPerDay');
  const estCostEl = document.getElementById('estCost');

  // Pricing: $0.40 per 1M tokens (approx midpoint for gpt-4.1-mini input/output)
  const COST_PER_MILLION_TOKENS = 0.4;

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
    try {
      const response = await fetch(`/api/metrics?limit_days=${limitDays}`);
      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }
      const data = await response.json();

      if (data.events && data.events.length > 0) {
        renderChart(data.events);
        updateSummaryMetrics(data.events, parseInt(limitDays, 10));
        showState('chart');
      } else {
        updateSummaryMetrics([], 0);
        showState('empty');
      }
    } catch (error) {
      console.error('Error fetching metrics:', error);
      updateSummaryMetrics([], 0);
      showState('error');
    }
  }

  function renderChart(events) {
    // Group events by metric name
    const metricsByName = {};
    for (const event of events) {
      if (!metricsByName[event.name]) {
        metricsByName[event.name] = [];
      }
      // Assume UTC time so we don't get off by one dates
      const [year, month, day] = event.timestamp.split('-').map(Number);
      const jsIsStupidMonth = month - 1;
      const utcMidnight = new Date(year, jsIsStupidMonth, day);
      metricsByName[event.name].push({
        timestamp: utcMidnight,
        value: event.value,
      });
    }
    console.log(metricsByName);

    // Sort each metric's events by timestamp
    for (const name in metricsByName) {
      metricsByName[name].sort((a, b) => a.timestamp - b.timestamp);
    }

    // Prepare data for echarts
    const series = [];
    const metricNames = Object.keys(metricsByName);

    if (metricNames.length === 0) {
      showState('empty');
      return;
    }

    // Find all unique timestamps across all metrics
    const allTimestamps = new Set();
    for (const name in metricsByName) {
      for (const event of metricsByName[name]) {
        allTimestamps.add(event.timestamp.getTime());
      }
    }

    // Create sorted array of timestamps
    const timeline = Array.from(allTimestamps).sort();

    // For each metric, create a series
    for (const name of metricNames) {
      const eventsMap = new Map(
        metricsByName[name].map((e) => [e.timestamp.getTime(), e.value]),
      );

      const data = timeline.map((ts) => eventsMap.get(ts) || null);

      series.push({
        name: formatMetricName(name),
        type: 'line',
        data: data,
        smooth: true,
        connectNulls: false,
      });
    }

    // Format timestamps for x-axis
    const xAxisData = timeline.map((ts) => {
      const date = new Date(ts);
      return `${date.getMonth() + 1}/${date.getDate()}`;
    });

    // Initialize or update chart
    if (chartInstance) {
      chartInstance.dispose();
    }

    chartInstance = echarts.init(chartDiv);
    const option = {
      tooltip: {
        trigger: 'axis',
        axisPointer: {
          type: 'cross',
        },
      },
      legend: {
        data: series.map((s) => s.name),
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
      yAxis: {
        type: 'value',
      },
      series: series,
    };

    chartInstance.setOption(option);

    // Handle window resize
    window.addEventListener('resize', () => {
      if (chartInstance) {
        chartInstance.resize();
      }
    });
  }

  function formatMetricName(name) {
    // Convert kebab-case to Title Case
    return name
      .split('-')
      .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
      .join(' ');
  }

  function updateSummaryMetrics(events, _limitDays) {
    // Filter for token-count events
    const tokenEvents = events.filter((e) => e.name === 'token-count');

    if (tokenEvents.length === 0) {
      totalTokensEl.textContent = '--';
      avgTokensPerDayEl.textContent = '--';
      estCostEl.textContent = '--';
      return;
    }

    // Sum all token values
    const totalTokens = tokenEvents.reduce((sum, e) => sum + e.value, 0);

    // Calculate average per day (use the actual number of days with data)
    const uniqueDays = new Set(tokenEvents.map((e) => e.timestamp));
    const daysWithData = uniqueDays.size;
    const avgTokensPerDay = daysWithData > 0 ? totalTokens / daysWithData : 0;

    // Calculate estimated cost
    const estCost = (totalTokens / 1_000_000) * COST_PER_MILLION_TOKENS;

    // Format and display
    totalTokensEl.textContent = formatNumber(totalTokens);
    avgTokensPerDayEl.textContent = formatNumber(Math.round(avgTokensPerDay));
    estCostEl.textContent = `$${estCost.toFixed(2)}`;
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
  retryBtn.addEventListener('click', fetchMetrics);

  // Initial load
  fetchMetrics().then(() => chartInstance.resize());
});
