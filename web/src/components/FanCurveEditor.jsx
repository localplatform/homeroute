import { useState, useRef, useCallback, useEffect } from 'react';

const TEMP_MIN = 30;
const TEMP_MAX = 70;
const PWM_MIN = 0;
const PWM_MAX = 100;

const GRAPH_WIDTH = 400;
const GRAPH_HEIGHT = 250;
const PADDING = { top: 20, right: 30, bottom: 40, left: 50 };

const INNER_WIDTH = GRAPH_WIDTH - PADDING.left - PADDING.right;
const INNER_HEIGHT = GRAPH_HEIGHT - PADDING.top - PADDING.bottom;

// Convert data coordinates to SVG coordinates
function dataToSvg(temp, pwm) {
  const x = PADDING.left + ((temp - TEMP_MIN) / (TEMP_MAX - TEMP_MIN)) * INNER_WIDTH;
  const y = PADDING.top + INNER_HEIGHT - ((pwm - PWM_MIN) / (PWM_MAX - PWM_MIN)) * INNER_HEIGHT;
  return { x, y };
}

// Convert SVG coordinates to data coordinates
function svgToData(x, y) {
  const temp = TEMP_MIN + ((x - PADDING.left) / INNER_WIDTH) * (TEMP_MAX - TEMP_MIN);
  const pwm = PWM_MAX - ((y - PADDING.top) / INNER_HEIGHT) * (PWM_MAX - PWM_MIN);
  return {
    temp: Math.round(Math.max(TEMP_MIN, Math.min(TEMP_MAX, temp))),
    pwm: Math.round(Math.max(PWM_MIN, Math.min(PWM_MAX, pwm)))
  };
}

// Generate SVG path from curve points
function curvePath(points) {
  if (!points || points.length === 0) return '';

  const svgPoints = points.map(([temp, pwm]) => dataToSvg(temp, pwm));
  return svgPoints.map((p, i) => `${i === 0 ? 'M' : 'L'} ${p.x} ${p.y}`).join(' ');
}

function FanCurveEditor({ profiles, activeProfile, onCurveChange, onDragEnd }) {
  const svgRef = useRef(null);
  const [dragging, setDragging] = useState(null); // { fan: 'fan1', pointIndex: 0 }
  const [hoveredPoint, setHoveredPoint] = useState(null);

  const profile = profiles?.find(p => p.name === activeProfile);
  const fan1Curve = profile?.fans?.fan1?.curve || [[30, 35], [50, 55], [70, 100]];
  const fan2Curve = profile?.fans?.fan2?.curve || [[30, 30], [50, 50], [70, 90]];

  const handleMouseDown = useCallback((e, fan, pointIndex) => {
    e.preventDefault();
    setDragging({ fan, pointIndex });
  }, []);

  const handleMouseMove = useCallback((e) => {
    if (!dragging || !svgRef.current) return;

    const svg = svgRef.current;
    const rect = svg.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;

    const { temp, pwm } = svgToData(x, y);

    // Get current curve
    const currentCurve = dragging.fan === 'fan1' ? [...fan1Curve] : [...fan2Curve];
    const point = currentCurve[dragging.pointIndex];

    // Constrain temperature to be between neighbors
    let minTemp = TEMP_MIN;
    let maxTemp = TEMP_MAX;
    if (dragging.pointIndex > 0) {
      minTemp = currentCurve[dragging.pointIndex - 1][0] + 1;
    }
    if (dragging.pointIndex < currentCurve.length - 1) {
      maxTemp = currentCurve[dragging.pointIndex + 1][0] - 1;
    }

    const constrainedTemp = Math.max(minTemp, Math.min(maxTemp, temp));
    currentCurve[dragging.pointIndex] = [constrainedTemp, pwm];

    onCurveChange(activeProfile, dragging.fan, currentCurve);
  }, [dragging, fan1Curve, fan2Curve, activeProfile, onCurveChange]);

  const handleMouseUp = useCallback(() => {
    if (dragging && onDragEnd) {
      onDragEnd(activeProfile);
    }
    setDragging(null);
  }, [dragging, activeProfile, onDragEnd]);

  useEffect(() => {
    if (dragging) {
      window.addEventListener('mousemove', handleMouseMove);
      window.addEventListener('mouseup', handleMouseUp);
      return () => {
        window.removeEventListener('mousemove', handleMouseMove);
        window.removeEventListener('mouseup', handleMouseUp);
      };
    }
  }, [dragging, handleMouseMove, handleMouseUp]);

  // Grid lines
  const gridLinesX = [30, 40, 50, 60, 70];
  const gridLinesY = [0, 25, 50, 75, 100];

  return (
    <div className="space-y-4">
      <svg
        ref={svgRef}
        viewBox={`0 0 ${GRAPH_WIDTH} ${GRAPH_HEIGHT}`}
        className="w-full max-w-lg bg-gray-900 rounded-lg"
        style={{ cursor: dragging ? 'grabbing' : 'default' }}
      >
        {/* Grid */}
        <g className="text-gray-700">
          {/* Vertical grid lines (temperature) */}
          {gridLinesX.map(temp => {
            const { x } = dataToSvg(temp, 0);
            return (
              <line
                key={`grid-x-${temp}`}
                x1={x}
                y1={PADDING.top}
                x2={x}
                y2={PADDING.top + INNER_HEIGHT}
                stroke="currentColor"
                strokeWidth="1"
                strokeOpacity="0.3"
              />
            );
          })}
          {/* Horizontal grid lines (PWM) */}
          {gridLinesY.map(pwm => {
            const { y } = dataToSvg(TEMP_MIN, pwm);
            return (
              <line
                key={`grid-y-${pwm}`}
                x1={PADDING.left}
                y1={y}
                x2={PADDING.left + INNER_WIDTH}
                y2={y}
                stroke="currentColor"
                strokeWidth="1"
                strokeOpacity="0.3"
              />
            );
          })}
        </g>

        {/* Axes */}
        <g>
          {/* X axis */}
          <line
            x1={PADDING.left}
            y1={PADDING.top + INNER_HEIGHT}
            x2={PADDING.left + INNER_WIDTH}
            y2={PADDING.top + INNER_HEIGHT}
            stroke="#6b7280"
            strokeWidth="2"
          />
          {/* Y axis */}
          <line
            x1={PADDING.left}
            y1={PADDING.top}
            x2={PADDING.left}
            y2={PADDING.top + INNER_HEIGHT}
            stroke="#6b7280"
            strokeWidth="2"
          />
        </g>

        {/* X axis labels */}
        <g className="text-gray-400 text-xs">
          {gridLinesX.map(temp => {
            const { x } = dataToSvg(temp, 0);
            return (
              <text
                key={`label-x-${temp}`}
                x={x}
                y={PADDING.top + INNER_HEIGHT + 20}
                textAnchor="middle"
                fill="currentColor"
                fontSize="11"
              >
                {temp}°
              </text>
            );
          })}
          <text
            x={PADDING.left + INNER_WIDTH / 2}
            y={GRAPH_HEIGHT - 5}
            textAnchor="middle"
            fill="#9ca3af"
            fontSize="12"
          >
            Température CPU (°C)
          </text>
        </g>

        {/* Y axis labels */}
        <g className="text-gray-400 text-xs">
          {gridLinesY.map(pwm => {
            const { y } = dataToSvg(TEMP_MIN, pwm);
            return (
              <text
                key={`label-y-${pwm}`}
                x={PADDING.left - 8}
                y={y + 4}
                textAnchor="end"
                fill="currentColor"
                fontSize="11"
              >
                {pwm}%
              </text>
            );
          })}
          <text
            x={15}
            y={PADDING.top + INNER_HEIGHT / 2}
            textAnchor="middle"
            fill="#9ca3af"
            fontSize="12"
            transform={`rotate(-90, 15, ${PADDING.top + INNER_HEIGHT / 2})`}
          >
            Vitesse PWM
          </text>
        </g>

        {/* Fan 2 curve (dashed, behind) */}
        <path
          d={curvePath(fan2Curve)}
          fill="none"
          stroke="#a855f7"
          strokeWidth="2"
          strokeDasharray="6 4"
          strokeLinecap="round"
        />

        {/* Fan 1 curve (solid, front) */}
        <path
          d={curvePath(fan1Curve)}
          fill="none"
          stroke="#3b82f6"
          strokeWidth="2.5"
          strokeLinecap="round"
        />

        {/* Fan 2 points */}
        {fan2Curve.map(([temp, pwm], i) => {
          const { x, y } = dataToSvg(temp, pwm);
          const isHovered = hoveredPoint?.fan === 'fan2' && hoveredPoint?.index === i;
          const isDragging = dragging?.fan === 'fan2' && dragging?.pointIndex === i;
          return (
            <g key={`fan2-point-${i}`}>
              <circle
                cx={x}
                cy={y}
                r={isDragging ? 10 : isHovered ? 8 : 6}
                fill={isDragging ? '#c084fc' : '#a855f7'}
                stroke="#1f2937"
                strokeWidth="2"
                style={{ cursor: 'grab' }}
                onMouseDown={(e) => handleMouseDown(e, 'fan2', i)}
                onMouseEnter={() => setHoveredPoint({ fan: 'fan2', index: i })}
                onMouseLeave={() => setHoveredPoint(null)}
              />
              {(isHovered || isDragging) && (
                <text
                  x={x}
                  y={y - 14}
                  textAnchor="middle"
                  fill="#c084fc"
                  fontSize="10"
                  fontWeight="bold"
                >
                  {temp}° → {pwm}%
                </text>
              )}
            </g>
          );
        })}

        {/* Fan 1 points */}
        {fan1Curve.map(([temp, pwm], i) => {
          const { x, y } = dataToSvg(temp, pwm);
          const isHovered = hoveredPoint?.fan === 'fan1' && hoveredPoint?.index === i;
          const isDragging = dragging?.fan === 'fan1' && dragging?.pointIndex === i;
          return (
            <g key={`fan1-point-${i}`}>
              <circle
                cx={x}
                cy={y}
                r={isDragging ? 10 : isHovered ? 8 : 6}
                fill={isDragging ? '#60a5fa' : '#3b82f6'}
                stroke="#1f2937"
                strokeWidth="2"
                style={{ cursor: 'grab' }}
                onMouseDown={(e) => handleMouseDown(e, 'fan1', i)}
                onMouseEnter={() => setHoveredPoint({ fan: 'fan1', index: i })}
                onMouseLeave={() => setHoveredPoint(null)}
              />
              {(isHovered || isDragging) && (
                <text
                  x={x}
                  y={y - 14}
                  textAnchor="middle"
                  fill="#60a5fa"
                  fontSize="10"
                  fontWeight="bold"
                >
                  {temp}° → {pwm}%
                </text>
              )}
            </g>
          );
        })}
      </svg>

      {/* Legend */}
      <div className="flex items-center justify-center gap-6 text-sm">
        <div className="flex items-center gap-2">
          <div className="w-6 h-0.5 bg-blue-500"></div>
          <span className="text-gray-300">CPU_FAN</span>
        </div>
        <div className="flex items-center gap-2">
          <div className="w-6 h-0.5 bg-purple-500" style={{ borderStyle: 'dashed', borderWidth: '1px 0 0 0' }}></div>
          <span className="text-gray-300">SYS_FAN</span>
        </div>
      </div>
    </div>
  );
}

export default FanCurveEditor;
