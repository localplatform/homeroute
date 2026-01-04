const TEMP_MIN = 30;
const TEMP_MAX = 70;
const PWM_MIN = 0;
const PWM_MAX = 100;

const WIDTH = 120;
const HEIGHT = 60;
const PADDING = { top: 5, right: 5, bottom: 5, left: 5 };

const INNER_WIDTH = WIDTH - PADDING.left - PADDING.right;
const INNER_HEIGHT = HEIGHT - PADDING.top - PADDING.bottom;

function dataToSvg(temp, pwm) {
  const x = PADDING.left + ((temp - TEMP_MIN) / (TEMP_MAX - TEMP_MIN)) * INNER_WIDTH;
  const y = PADDING.top + INNER_HEIGHT - ((pwm - PWM_MIN) / (PWM_MAX - PWM_MIN)) * INNER_HEIGHT;
  return { x, y };
}

function interpolatePwm(curve, temp) {
  if (!curve || curve.length === 0) return 50;
  if (temp <= curve[0][0]) return curve[0][1];
  if (temp >= curve[curve.length - 1][0]) return curve[curve.length - 1][1];

  for (let i = 0; i < curve.length - 1; i++) {
    if (temp >= curve[i][0] && temp <= curve[i + 1][0]) {
      const [t1, p1] = curve[i];
      const [t2, p2] = curve[i + 1];
      return p1 + ((temp - t1) / (t2 - t1)) * (p2 - p1);
    }
  }
  return 50;
}

function curvePath(points) {
  if (!points || points.length === 0) return '';
  const svgPoints = points.map(([temp, pwm]) => dataToSvg(temp, pwm));
  return svgPoints.map((p, i) => `${i === 0 ? 'M' : 'L'} ${p.x} ${p.y}`).join(' ');
}

function MiniCurveGraph({ profile, currentTemp }) {
  if (!profile?.fans?.fan1?.curve) return null;

  const fan1Curve = profile.fans.fan1.curve;
  const fan2Curve = profile.fans.fan2?.curve;

  // Calculate current PWM based on temperature
  const currentPwm1 = interpolatePwm(fan1Curve, currentTemp);
  const currentPwm2 = fan2Curve ? interpolatePwm(fan2Curve, currentTemp) : null;

  // Current temperature position
  const tempX = PADDING.left + ((Math.max(TEMP_MIN, Math.min(TEMP_MAX, currentTemp)) - TEMP_MIN) / (TEMP_MAX - TEMP_MIN)) * INNER_WIDTH;
  const pos1 = dataToSvg(currentTemp, currentPwm1);
  const pos2 = fan2Curve ? dataToSvg(currentTemp, currentPwm2) : null;

  return (
    <svg viewBox={`0 0 ${WIDTH} ${HEIGHT}`} className="w-full h-full">
      {/* Background */}
      <rect
        x={PADDING.left}
        y={PADDING.top}
        width={INNER_WIDTH}
        height={INNER_HEIGHT}
        fill="#1f2937"
        rx="4"
      />

      {/* Temperature vertical line */}
      {currentTemp !== null && (
        <line
          x1={tempX}
          y1={PADDING.top}
          x2={tempX}
          y2={PADDING.top + INNER_HEIGHT}
          stroke="#4b5563"
          strokeWidth="1"
          strokeDasharray="2 2"
        />
      )}

      {/* Fan 2 curve (dashed, behind) */}
      {fan2Curve && (
        <path
          d={curvePath(fan2Curve)}
          fill="none"
          stroke="#a855f7"
          strokeWidth="1.5"
          strokeDasharray="3 2"
          strokeLinecap="round"
        />
      )}

      {/* Fan 1 curve (solid, front) */}
      <path
        d={curvePath(fan1Curve)}
        fill="none"
        stroke="#3b82f6"
        strokeWidth="2"
        strokeLinecap="round"
      />

      {/* Current position markers */}
      {currentTemp !== null && (
        <>
          {/* Fan 2 marker */}
          {pos2 && (
            <circle
              cx={pos2.x}
              cy={pos2.y}
              r="4"
              fill="#a855f7"
              stroke="#1f2937"
              strokeWidth="1.5"
            />
          )}
          {/* Fan 1 marker */}
          <circle
            cx={pos1.x}
            cy={pos1.y}
            r="5"
            fill="#3b82f6"
            stroke="#1f2937"
            strokeWidth="1.5"
          />
        </>
      )}
    </svg>
  );
}

export default MiniCurveGraph;
