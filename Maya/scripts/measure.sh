#!/usr/bin/env bash
set -euo pipefail

pid=''
duration=600
interval=5
output='release/measurements.csv'
fixture='idle'
while (($#)); do
  case "$1" in
    --pid) pid=${2:?}; shift 2 ;;
    --duration-seconds) duration=${2:?}; shift 2 ;;
    --interval-seconds) interval=${2:?}; shift 2 ;;
    --output) output=${2:?}; shift 2 ;;
    --fixture) fixture=${2:?}; shift 2 ;;
    *) echo "measure: unknown argument: $1" >&2; exit 2 ;;
  esac
done
[[ "$pid" =~ ^[0-9]+$ ]] && [[ -r "/proc/$pid/stat" ]] || { echo 'measure: live --pid is required' >&2; exit 2; }
[[ "$duration" =~ ^[0-9]+([.][0-9]+)?$ ]] || { echo 'measure: invalid duration' >&2; exit 2; }
[[ "$interval" =~ ^[0-9]+([.][0-9]+)?$ ]] || { echo 'measure: invalid interval' >&2; exit 2; }

mkdir -p "$(dirname "$output")"
read_stat() {
  local line rest
  IFS= read -r line <"/proc/$pid/stat"
  rest=${line#*) }
  set -- $rest
  printf '%s %s\n' "$(( ${12} + ${13} ))" "${20}"
}
read -r previous_ticks expected_start < <(read_stat)
previous_ns=$(date +%s%N)
clock_ticks=$(getconf CLK_TCK)
boot_id=$(cat /proc/sys/kernel/random/boot_id 2>/dev/null || printf 'unavailable')
printf 'timestamp_utc,fixture,pid,start_ticks,boot_id,category,rss_bytes,cpu_percent_one_core,status\n' >"$output"

end_ns=$(awk -v now="$previous_ns" -v seconds="$duration" 'BEGIN{printf "%.0f",now+(seconds*1000000000)}')
while [[ $(date +%s%N) -lt $end_ns ]]; do
  sleep "$interval"
  if [[ ! -r "/proc/$pid/stat" ]]; then
    printf '%s,%s,%s,%s,%s,core,,,%s\n' "$(date -u +%FT%TZ)" "$fixture" "$pid" "$expected_start" "$boot_id" 'process_exited' >>"$output"
    exit 1
  fi
  read -r ticks start_ticks < <(read_stat)
  [[ "$start_ticks" == "$expected_start" ]] || { echo 'measure: PID identity changed' >&2; exit 1; }
  now_ns=$(date +%s%N)
  rss_kib=$(awk '/^VmRSS:/{print $2}' "/proc/$pid/status")
  rss_bytes=$(( ${rss_kib:-0} * 1024 ))
  cpu=$(awk -v dticks="$((ticks-previous_ticks))" -v hz="$clock_ticks" -v dns="$((now_ns-previous_ns))" 'BEGIN{if(dns<=0){print ""}else{printf "%.6f",100*dticks/hz/(dns/1000000000)}}')
  printf '%s,%s,%s,%s,%s,core,%s,%s,ok\n' "$(date -u +%FT%TZ)" "$fixture" "$pid" "$start_ticks" "$boot_id" "$rss_bytes" "$cpu" >>"$output"
  previous_ticks=$ticks
  previous_ns=$now_ns
done
echo "measure: wrote $output"
