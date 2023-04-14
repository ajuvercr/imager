#!/bin/bash
trap "jobs -p | xargs kill ; trap - INT" INT

if [ -f ./target/release/imager ]; then
  IMAGER=./target/release/imager
elif [ -f ./target/debug/imager ]; then
  IMAGER=./target/debug/imager
else
  IMAGER=imager
fi

displays="$(xrandr --listactivemonitors | grep -v Monitors | cut -sd' ' -f4 | sed 's/\([0-9]*\)\/[0-9]*x\([0-9]*\)\/[0-9]*+\([0-9]*\)+\([0-9]*\).*/-w \1 --height \2 -x \3 -y \4/')"

echo $displays
echo $IMAGER

while IFS= read -r line; do
  $IMAGER $line $@ &
done <<< $displays

wait
