#!/bin/sh

curr_path=`pwd`
script_full_name=$BASH_SOURCE
cd `dirname $script_full_name`
script_path=`pwd`
cd $curr_path

nohup $script_path/object-storage-bin $script_path/config.json5 &
