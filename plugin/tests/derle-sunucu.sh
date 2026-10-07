#!/bin/bash
# Sunucuda (CT 970): /root/plugin.tgz (plugin klasörü) açılır, gcc:14 kabında derlenir.
# usage: pb.sh <extra g++ args...>
rm -rf /root/pl && mkdir /root/pl && tar xzf /root/plugin.tgz -C /root/pl --no-same-owner
docker run --rm --network host -v /root/pl/plugin:/p -w /p gcc:14 sh -c "g++ -std=c++17 -O1 -g -Wall -Wextra -Werror -Wno-missing-field-initializers -DFXVPS_PLUGIN_VERSION='\"0.1.0\"' $*" 2>&1
