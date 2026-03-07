import { useState } from 'react';

interface Props {
    rows: string[][];
    kind: 'register' | 'balance-interval';
}

const COLORS = [
    '#1f77b4',
    '#ff7f0e',
    '#2ca02c',
    '#d62728',
    '#9467bd',
    '#8c564b',
    '#e377c2',
    '#7f7f7f',
    '#bcbd22',
    '#17becf',
    '#aec7e8',
    '#ffbb78',
    '#98df8a',
    '#ff9896',
    '#c5b0d5',
];

function parseAmount(s: string): number | null {
    const n = parseFloat(s.replace(/[^0-9.-]/g, ''));
    return isFinite(n) ? n : null;
}

function formatY(v: number): string {
    const abs = Math.abs(v);
    if (abs >= 1e6) return `${(v / 1e6).toFixed(1)}M`;
    if (abs >= 1e3) return `${(v / 1e3).toFixed(1)}k`;
    return v.toFixed(0);
}

const SVG_W = 800;
const SVG_H = 200;
const ML = 65;
const MR = 20;
const MT = 10;
const MB = 50;
const PW = SVG_W - ML - MR;
const PH = SVG_H - MT - MB;

interface TooltipState {
    x: number;
    y: number;
    content: string;
}

export function ReportChart({ rows, kind }: Props) {
    if (rows.length < 2) return null;
    return kind === 'register' ? (
        <RegisterChart rows={rows} />
    ) : (
        <BalanceIntervalChart rows={rows} />
    );
}

function RegisterChart({ rows }: { rows: string[][] }) {
    const [tooltip, setTooltip] = useState<TooltipState | null>(null);

    const header = rows[0] ?? [];
    const dateIdx = header.indexOf('date');
    const totalIdx =
        header.indexOf('total') !== -1
            ? header.indexOf('total')
            : header.indexOf('balance');
    const amountIdx = header.indexOf('amount');
    const descIdx = header.indexOf('description');
    const txnidxIdx = header.indexOf('txnidx');

    if (dateIdx === -1 || totalIdx === -1) return null;

    interface Point {
        dateStr: string;
        totalStr: string;
        ms: number; // noon-UTC timestamp, matching hledger-web's dayToUtcNoonTimestamp
        y: number;
        amount: string;
        desc: string;
    }

    // hledger register outputs one row per posting. Group by txnidx and keep
    // only the FIRST row per transaction.
    //
    // hledger-web uses accountTransactionsReport filtered to one account, so
    // it gets exactly one item per transaction with the correct running balance.
    // From CSV, when the user has filtered to one account (e.g. "register
    // acct:Assets:Checking"), there is also exactly one row per txnidx and
    // its `total` is the running balance of that account. Keeping the first
    // row is correct for that case.
    //
    // For an unfiltered register, multiple postings per txnidx share the same
    // txnidx; keeping the FIRST (not last) avoids the all-zeros problem: the
    // last posting of a balanced transaction always brings total back to 0,
    // while the first posting is non-zero.
    const firstPerTxn = new Map<string, Point>();
    for (const row of rows.slice(1)) {
        const dateStr = row[dateIdx] ?? '';
        const totalStr = row[totalIdx] ?? '';
        const y = parseAmount(totalStr);
        if (!dateStr || y === null) continue;
        // noon UTC, matching hledger-web's dayToUtcNoonTimestamp
        const ms = new Date(dateStr + 'T12:00:00Z').getTime();
        if (!isFinite(ms)) continue;
        const key = txnidxIdx !== -1 ? (row[txnidxIdx] ?? dateStr) : dateStr;
        if (!firstPerTxn.has(key)) {
            firstPerTxn.set(key, {
                dateStr,
                totalStr,
                ms,
                y,
                amount: row[amountIdx] ?? '',
                desc: row[descIdx] ?? '',
            });
        }
    }
    const points = [...firstPerTxn.values()];

    if (points.length === 0) return null;

    // X scale: proportional to real time, like hledger-web's time-mode x-axis
    const msValues = points.map((p) => p.ms);
    const minMs = Math.min(...msValues);
    const maxMs = Math.max(...msValues);
    const xScale = (ms: number): number =>
        minMs === maxMs ? PW / 2 : ((ms - minMs) / (maxMs - minMs)) * PW;

    // "now" divides past from future, matching hledger-web's grid markings
    const nowMs = (() => {
        const d = new Date();
        d.setUTCHours(12, 0, 0, 0);
        return d.getTime();
    })();
    const nowX = Math.max(0, Math.min(PW, xScale(nowMs)));

    // Y scale
    const rawMin = Math.min(...points.map((p) => p.y));
    const rawMax = Math.max(...points.map((p) => p.y));
    const rawRange = rawMax - rawMin;
    const pad = rawRange * 0.1 || 1;
    const yMin = rawMin - pad;
    const yMax = rawMax + pad;
    const yRange = yMax - yMin;

    const yScale = (v: number): number => PH - ((v - yMin) / yRange) * PH;
    const zeroY = Math.max(0, Math.min(PH, yScale(0)));

    // Stepped line path (hledger-web: lines.steps=true)
    let linePath = `M ${xScale(points[0]?.ms ?? 0)} ${yScale(points[0]?.y ?? 0)}`;
    for (let i = 1; i < points.length; i++) {
        linePath += ` H ${xScale(points[i]?.ms ?? 0)} V ${yScale(points[i]?.y ?? 0)}`;
    }

    // Y-axis ticks
    const Y_TICKS = 4;
    const yLabels = Array.from({ length: Y_TICKS + 1 }, (_, i) => {
        const v = yMin + (yRange * i) / Y_TICKS;
        return { v, y: yScale(v) };
    });

    // X-axis labels: sample by minimum SVG-unit spacing to avoid crowding
    const MIN_X_SPACING = 70;
    const xLabels: { dateStr: string; x: number }[] = [];
    let lastLabelX = -Infinity;
    for (const p of points) {
        const x = xScale(p.ms);
        if (x - lastLabelX >= MIN_X_SPACING) {
            xLabels.push({ dateStr: p.dateStr, x });
            lastLabelX = x;
        }
    }

    return (
        <div className="report-chart">
            <svg viewBox={`0 0 ${SVG_W} ${SVG_H}`} width="100%">
                <g transform={`translate(${ML},${MT})`}>
                    {/*
                     * Grid markings matching hledger-web's flot markings:
                     *   past   + negative → #ffdddd
                     *   future + positive → #e0e0e0
                     *   future + negative → #e8c8c8
                     *   past   + positive → (no fill, default background)
                     */}
                    {zeroY < PH && nowX > 0 && (
                        <rect
                            x={0}
                            y={zeroY}
                            width={nowX}
                            height={PH - zeroY}
                            fill="#ffdddd"
                        />
                    )}
                    {zeroY > 0 && nowX < PW && (
                        <rect
                            x={nowX}
                            y={0}
                            width={PW - nowX}
                            height={zeroY}
                            fill="#e0e0e0"
                        />
                    )}
                    {zeroY < PH && nowX < PW && (
                        <rect
                            x={nowX}
                            y={zeroY}
                            width={PW - nowX}
                            height={PH - zeroY}
                            fill="#e8c8c8"
                        />
                    )}

                    {/* Zero line: #bb0000, lineWidth 1 (matches hledger-web) */}
                    <line
                        x1={0}
                        y1={zeroY}
                        x2={PW}
                        y2={zeroY}
                        stroke="#bb0000"
                        strokeWidth={1}
                    />

                    {/* Stepped line (hledger-web series 1: lines.steps=true, points.show=false) */}
                    <path
                        d={linePath}
                        fill="none"
                        stroke={COLORS[0]}
                        strokeWidth={2}
                    />

                    {/*
                     * Hoverable points (hledger-web series 2: lines.show=false, points.show=true)
                     * Tooltip: "{balance} balance on {date} after {amount} posted by transaction:\n{desc}"
                     */}
                    {points.map((p, i) => (
                        <circle
                            key={i}
                            cx={xScale(p.ms)}
                            cy={yScale(p.y)}
                            r={3}
                            fill={COLORS[0]}
                            style={{ cursor: 'pointer' }}
                            onMouseEnter={(e) => {
                                setTooltip({
                                    x: e.clientX + 12,
                                    y: e.clientY - 32,
                                    content: `${p.totalStr} balance on ${p.dateStr} after ${p.amount} posted by transaction:\n${p.desc}`,
                                });
                            }}
                            onMouseLeave={() => {
                                setTooltip(null);
                            }}
                        />
                    ))}

                    {/* Y-axis labels */}
                    {yLabels.map(({ v, y }) => (
                        <text
                            key={v}
                            x={-8}
                            y={y}
                            textAnchor="end"
                            dominantBaseline="middle"
                            fontSize={11}
                            fill="#666"
                        >
                            {formatY(v)}
                        </text>
                    ))}

                    {/* X-axis date labels */}
                    {xLabels.map(({ dateStr, x }) => (
                        <text
                            key={dateStr}
                            x={x}
                            y={PH + 16}
                            textAnchor="middle"
                            fontSize={10}
                            fill="#666"
                        >
                            {dateStr}
                        </text>
                    ))}

                    {/* Axes */}
                    <line
                        x1={0}
                        y1={0}
                        x2={0}
                        y2={PH}
                        stroke="#d0d0d0"
                        strokeWidth={1}
                    />
                    <line
                        x1={0}
                        y1={PH}
                        x2={PW}
                        y2={PH}
                        stroke="#d0d0d0"
                        strokeWidth={1}
                    />
                </g>
            </svg>
            {tooltip !== null && (
                <div
                    className="report-chart-tooltip"
                    style={{ left: tooltip.x, top: tooltip.y }}
                >
                    {tooltip.content}
                </div>
            )}
        </div>
    );
}

function BalanceIntervalChart({ rows }: { rows: string[][] }) {
    const [tooltip, setTooltip] = useState<TooltipState | null>(null);

    const header = rows[0] ?? [];

    // Collect period column indices (skip col 0 "account" and "total"/"average")
    const periodCols: number[] = [];
    for (let i = 1; i < header.length; i++) {
        const h = (header[i] ?? '').toLowerCase();
        if (h !== '' && h !== 'total' && h !== 'average') {
            periodCols.push(i);
        }
    }
    const periods = periodCols.map((i) => header[i] ?? '');

    interface Series {
        account: string;
        values: (number | null)[];
    }

    const allSeries: Series[] = [];
    for (const row of rows.slice(1)) {
        // Skip section-header rows (all cells after first are empty)
        if (row.slice(1).every((c) => c === '')) continue;
        const account = row[0] ?? '';
        if (!account) continue;
        const values = periodCols.map((i) => parseAmount(row[i] ?? ''));
        allSeries.push({ account, values });
    }

    const MAX_SERIES = 15;
    const series = allSeries.slice(0, MAX_SERIES);
    const extraCount = allSeries.length - series.length;

    if (series.length === 0 || periods.length === 0) return null;

    const allValues = series.flatMap((s) =>
        s.values.filter((v): v is number => v !== null),
    );
    if (allValues.length === 0) return null;

    const rawMin = Math.min(...allValues);
    const rawMax = Math.max(...allValues);
    const rawRange = rawMax - rawMin;
    const pad = rawRange * 0.1 || 1;
    const yMin = rawMin - pad;
    const yMax = rawMax + pad;
    const yRange = yMax - yMin;

    const xScale = (i: number): number =>
        periods.length <= 1 ? PW / 2 : (i / (periods.length - 1)) * PW;
    const yScale = (v: number): number => PH - ((v - yMin) / yRange) * PH;
    const zeroY = Math.max(0, Math.min(PH, yScale(0)));

    const Y_TICKS = 4;
    const yLabels = Array.from({ length: Y_TICKS + 1 }, (_, i) => {
        const v = yMin + (yRange * i) / Y_TICKS;
        return { v, y: yScale(v) };
    });

    const MAX_X_LABELS = 8;
    const xStep = Math.max(1, Math.ceil(periods.length / MAX_X_LABELS));
    const showXIdx = new Set<number>();
    for (let i = 0; i < periods.length; i += xStep) showXIdx.add(i);
    showXIdx.add(periods.length - 1);

    return (
        <div className="report-chart">
            {extraCount > 0 && (
                <p
                    style={{
                        fontSize: '0.78rem',
                        color: '#888',
                        margin: '0 0 0.25rem',
                    }}
                >
                    Showing {MAX_SERIES} of {allSeries.length} accounts
                </p>
            )}
            <svg viewBox={`0 0 ${SVG_W} ${SVG_H}`} width="100%">
                <g transform={`translate(${ML},${MT})`}>
                    {/* Zero line */}
                    <line
                        x1={0}
                        y1={zeroY}
                        x2={PW}
                        y2={zeroY}
                        stroke="#cc0000"
                        strokeWidth={1}
                        strokeDasharray="4,3"
                        opacity={0.6}
                    />

                    {/* Series polylines */}
                    {series.map((s, si) => {
                        const color =
                            COLORS[si % COLORS.length] ??
                            COLORS[0] ??
                            '#1f77b4';
                        const pts: string[] = [];
                        s.values.forEach((v, pi) => {
                            if (v !== null)
                                pts.push(`${xScale(pi)},${yScale(v)}`);
                        });
                        if (pts.length < 1) return null;
                        return (
                            <polyline
                                key={si}
                                points={pts.join(' ')}
                                fill="none"
                                stroke={color}
                                strokeWidth={2}
                            />
                        );
                    })}

                    {/* Data points with tooltip */}
                    {series.map((s, si) =>
                        s.values.map((v, pi) => {
                            if (v === null) return null;
                            const color =
                                COLORS[si % COLORS.length] ??
                                COLORS[0] ??
                                '#1f77b4';
                            return (
                                <circle
                                    key={`${si}-${pi}`}
                                    cx={xScale(pi)}
                                    cy={yScale(v)}
                                    r={3}
                                    fill={color}
                                    style={{ cursor: 'pointer' }}
                                    onMouseEnter={(e) => {
                                        setTooltip({
                                            x: e.clientX + 12,
                                            y: e.clientY - 32,
                                            content: `${periods[pi]} — ${s.account}: ${v.toLocaleString()}`,
                                        });
                                    }}
                                    onMouseLeave={() => {
                                        setTooltip(null);
                                    }}
                                />
                            );
                        }),
                    )}

                    {/* Y-axis labels */}
                    {yLabels.map(({ v, y }) => (
                        <text
                            key={v}
                            x={-8}
                            y={y}
                            textAnchor="end"
                            dominantBaseline="middle"
                            fontSize={11}
                            fill="#666"
                        >
                            {formatY(v)}
                        </text>
                    ))}

                    {/* X-axis period labels */}
                    {periods.map((p, i) => {
                        if (!showXIdx.has(i)) return null;
                        return (
                            <text
                                key={p}
                                x={xScale(i)}
                                y={PH + 16}
                                textAnchor="middle"
                                fontSize={10}
                                fill="#666"
                            >
                                {p}
                            </text>
                        );
                    })}

                    {/* Axes */}
                    <line
                        x1={0}
                        y1={0}
                        x2={0}
                        y2={PH}
                        stroke="#d0d0d0"
                        strokeWidth={1}
                    />
                    <line
                        x1={0}
                        y1={PH}
                        x2={PW}
                        y2={PH}
                        stroke="#d0d0d0"
                        strokeWidth={1}
                    />
                </g>
            </svg>

            {/* Legend */}
            <div className="report-chart-legend">
                {series.map((s, si) => (
                    <span key={si} className="report-chart-legend-item">
                        <span
                            className="report-chart-legend-swatch"
                            style={{
                                background: COLORS[si % COLORS.length],
                            }}
                        />
                        {s.account}
                    </span>
                ))}
            </div>

            {tooltip !== null && (
                <div
                    className="report-chart-tooltip"
                    style={{ left: tooltip.x, top: tooltip.y }}
                >
                    {tooltip.content}
                </div>
            )}
        </div>
    );
}
