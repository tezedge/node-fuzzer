# Copyright(c) SimpleStaking, Viable Systems and Tezedge Contributors
# SPDX-License-Identifier: MIT

import json
import pathlib
from datetime import datetime
import os

kcov_index_html_template = """
<html>
<head>
  <title id="window-title">???</title>
  <link rel="stylesheet" href="/web-files/tablesorter-theme.css">
  <link rel="stylesheet" type="text/css" href="/web-files/bcov.css"/>
</head>
<noscript>
<font color=red><B>ERROR:</B></font> JavaScript need to be enabled for the coverage report to work.
</noscript>
<script type="text/javascript" src="index.js"></script>
<script type="text/javascript" src="/web-files/js/jquery.min.js"></script>
<script type="text/javascript" src="/web-files/js/tablesorter.min.js"></script>
<script type="text/javascript" src="/web-files/js/jquery.tablesorter.widgets.min.js"></script>
<script type="text/javascript" src="/web-files/js/handlebars.js"></script>
<script type="text/javascript" src="/web-files/js/kcov.js"></script>
<body>
<table width="100%" border="0" cellspacing="0" cellpadding="0">
  <tr><td class="title">Coverage Report</td></tr>
  <tr><td class="ruler"><img src="data/glass.png" width="3" height="3" alt=""/></td></tr>
  <tr>
    <td width="100%">
      <table cellpadding="1" border="0" width="100%">
        <tr id="command">
          <td class="headerItem" width="20%">Command:</td>
          <td id="header-command" class="headerValue" width="80%" colspan=6>???</td>
        </tr>
        <tr>
          <td class="headerItem" width="20%">Date: </td>
          <td id="header-date" class="headerValue" width="15%"></td>
          <td width="5%"></td>
          <td class="headerItem" width="20%">Instrumented lines:</td>
          <td id="header-instrumented" class="headerValue" width="10%">???</td>
        </tr>
        <tr>
          <td class="headerItem" width="20%">Code covered:</td>
          <td id="header-percent-covered" width="15%">???</td>
          <td width="5%"></td>
          <td class="headerItem" width="20%">Executed lines:</td>
          <td id="header-covered" class="headerValue" width="10%">???</td>
        </tr>
      </table>
    </td>
  </tr>
  <tr><td class="ruler"><img src="data/glass.png" width="3" height="3" alt=""/></td></tr>
</table>
<script id="files-template" type="text/x-handlebars-template">
<center>
  <table width="80%" cellpadding="2" cellspacing="1" border="0" id="index-table" class="tablesorter">
    <thead>
    <tr>
      <th class="tableHead" width="50%">Filename</th>
      <th width="20%">Coverage percent</th>
      <th width="10%">Covered lines</th>
      <th width="10%">Uncovered lines</th>
      <th width="10%">Executable lines</th>
    </tr>
    </thead>
    <tbody id="main-data">
    {{#each files}}
    <tr>
      <td class="coverFile"><a href="{{link}}" title="{{title}}">{{summary_name}}</a></td>
      <td class="coverPer"><span style="display:block;width:{{covered}}%" class="{{covered_class}}">{{covered}}%</td>
      <td class="coverNum">{{covered_lines}}</td>
      <td class="coverNum">{{uncovered_lines}}</td>
      <td class="coverNum">{{total_lines}}</td>
    </tr>
    {{/each}}
    </tbody>
    {{#each merged_files}}
    <tbody tablesorter-no-sort id="merged-data">
    <tr>
      <td class="coverFile"><a href="{{link}}" title="{{title}}">{{summary_name}}</a></td>
      <td class="coverPer"><span style="display:block;width:{{covered}}%" class="{{covered_class}}">{{covered}}%</td>
      <td class="coverNum">{{covered_lines}}</td>
      <td class="coverNum">{{uncovered_lines}}</td>
      <td class="coverNum">{{total_lines}}</td>
    </tr>
    </tbody>
    {{/each}}
  </table>
</center>
</script>
<div id="files-placeholder"></div>
<br>
<table width="100%" border="0" cellspacing="0" cellpadding="0">
  <tr><td class="ruler"><img src="data/amber.png" width="3" height="3" alt=""/></td></tr>
  <tr><td class="versionInfo">Generated by: <a href="http://simonkagstrom.github.com/kcov/index.html">Kcov</a></td></tr>
</table>
</body>
</html>
"""

kcov_html_template = """
<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN">
<html>
<head>
  <meta charset="utf-8">
  <title id="window-title">???</title>
  <link rel="stylesheet" type="text/css" href="/web-files/bcov.css"/>
</head>
<script type="text/javascript" src="/web-files/js/jquery.min.js"></script>
<script type="text/javascript" src="/web-files/js/handlebars.js"></script>
<script type="text/javascript" src="/web-files/js/kcov.js"></script>
<body>
<table width="100%" border="0" cellspacing="0" cellpadding="0">
  <tr><td class="title">Coverage Report</td></tr>
  <tr><td class="ruler"><img src="/web-files/data/glass.png" width="3" height="3" alt=""/></td></tr>
  <tr>
    <td width="100%">
      <table cellpadding="1" border="0" width="100%">
        <tr id="command">
          <td class="headerItem" width="20%">Command:</td>
          <td id="header-command" class="headerValue" width="60%" colspan=4>???</td>
          <td><span class="lineNumLegend">Line number</span></td>
        </tr>
        <tr>
          <td class="headerItem" width="20%">Date: </td>
          <td id="header-date" class="headerValue" width="20%" colspan=2></td>
          <td class="headerItem" width="20%">Instrumented lines:</td>
          <td id="header-instrumented" class="headerValue" width="20%">???</td>
          <td><span class="coverHitsLegend">Hits</span></td>
        </tr>
        <tr>
          <td class="headerItem" width="20%">Code covered:</td>
          <td id="header-percent-covered" width="20%" colspan=2>???</td>
          <td class="headerItem" width="20%">Executed lines:</td>
          <td id="header-covered" class="headerValue" width="20%">???</td>
          <td><span class="orderNumLegend">Order</span></td>
        </tr>
      </table>
    </td>
  </tr>
  <tr><td class="ruler"><img src="/web-files/data/glass.png" width="3" height="3" alt=""/></td></tr>
</table>
<script id="lines-template" type="text/x-handlebars-template">
<pre class="source" id="main-data">
{{#each lines}}
{{#ifCond possible_hits "==" "0"}}
<source-line><span class="lineNum">{{lineNum}}</span><span class="coverHits">{{hits}} </span><span class="{{class}}"> {{line}}</span></span><span class="orderNum"> {{order}}</span></source-line>
{{else}}
 {{#exists possible_hits}}
<source-line><span class="lineNum">{{lineNum}}</span><span class="coverHits">{{hits}} / {{possible_hits}}</span><span class="{{class}}"> {{line}}</span><span class="orderNum"> {{order}}</span></source-line>
 {{else}}
   {{#exists class}}
<source-line><span class="lineNum">{{lineNum}}</span><span class="coverHits">&nbsp;</span><span class="{{class}}"> {{line}}</span></span><span class="orderNum">&nbsp;</span></source-line>
   {{else}}
<source-line><span class="lineNum">{{lineNum}}</span><span class="coverHits">&nbsp;</span><span class="{{class}}"> {{line}}</span><span class="orderNum">&nbsp;</span></source-line>
   {{/exists}}
 {{/exists}}
{{/ifCond}}
{{/each}}
</pre>
</script>
<div id="lines-placeholder"></div>
<table width="100%" border="0" cellspacing="0" cellpadding="0" id="merged-data">
  <tr><td class="ruler"><img src="/web-files/data/amber.png" width="3" height="3" alt=""/></td></tr>
  <tr><td class="versionInfo">Generated by: <a href="http://simonkagstrom.github.com/kcov/index.html">Kcov</a></td></tr>
</table>
</body>
</html>
"""


def read_coverage():
    ret = []
    path = next(pathlib.Path('/tezos/_coverage_output').glob('*.coverage'))

    with open(path, 'r') as f:
        cov_info = f.read()
        cov_info = cov_info[len('BISECT-COVERAGE-4'):]
        parts = cov_info.lstrip().split(' ', 1)
        list_len = int(parts[0])
        cov_info = parts[1]

        while len(ret) != list_len:
            parts = cov_info.lstrip().split(' ', 1)
            str_len = int(parts[0])
            path_str = parts[1][:str_len]
            cov_info = parts[1][str_len:]
            parts = cov_info.lstrip().split(' ', 1)
            points_len = int(parts[0])

            if points_len > 0:
                parts = parts[1].split(' ', points_len)
                points = map(int, parts[:-1])
            else:
                points = []

            cov_info = parts[-1]
            parts = cov_info.lstrip().split(' ', 1)
            counts_len = int(parts[0])

            if counts_len > 0:
                parts = parts[1].split(' ', counts_len)
                counts = map(int, parts[:-1])
            else:
                counts = []

            cov_info = parts[-1]
            ret.append((path_str, tuple(zip(points, counts))))

    return ret


root = '/coverage/develop/.fuzzing.latest/p2p-rpc-fuzzers/ocaml/'
date = str(datetime.now()).split('.')[0]
files = dict()
index = []
index_header = {
    'command': 'OCaml coverage',
    'covered': 0,
    'instrumented': 0,
    'date': date
}

for file, counts in read_coverage():
    offsets = dict()

    if file not in files:
        files[file] = offsets
    else:
        offsets = files[file]

    for pos, counter in counts:
        offsets[pos] = counter != 0

for (file, offsets) in files.items():
    lines_info = []
    covered = 0
    instrumented = 0
    path = pathlib.Path(root + file)
    path.parents[0].mkdir(parents=True, exist_ok=True)

    with open(f'/tezos/_build/default/{file}', 'r') as f:
        lines = dict()

        with open(f'/tezos/_build/default/{file}', 'rb') as fb:
            data = fb.read()

            for offset in offsets:
                line = data[:offset].decode().count('\n') + 1

                if line not in lines:
                    lines[line] = offsets[offset]
                else:
                    lines[line] |= offsets[offset]

        print(file)

        for line_number, code in enumerate(f.readlines(), start=1):
            line_info = dict()
            line_info['lineNum'] = str(line_number)
            line_info['line'] = code

            if line_number in lines:
                # print(line_number)
                # print(lines)
                instrumented += 1
                line_info['order'] = '0'
                line_info['possible_hits'] = '1'

                if lines[line_number] is True:
                    covered += 1
                    line_info['hits'] = '1'
                    line_info['class'] = 'lineCov'
                else:
                    line_info['hits'] = '0'
                    line_info['class'] = 'lineNoCov'

            lines_info.append(line_info)

        header = {
            'command': 'OCaml coverage',
            'covered': covered,
            'instrumented': instrumented,
            'date': date
        }

        if instrumented == 0:
            continue

        covered_per = (covered / instrumented) * 100
        index.append({
            'title': path.name,
            'link': f'{file}.kcov.html',
            'summary_name': f'[...]{file}',
            'covered': f'{covered_per:.1f}',
            'covered_lines': str(covered),
            'uncovered_lines': str(instrumented - covered),
            'total_lines': str(instrumented),
        })

        index_header['covered'] += header['covered']
        index_header['instrumented'] += header['instrumented']

        if covered_per < 25:
            index[-1]['covered_class'] = 'lineNoCov'
        elif covered_per < 75:
            index[-1]['covered_class'] = 'linePartCov'
        else:
            index[-1]['covered_class'] = 'lineCov'

    with open(f'{str(path)}.js', 'w') as js_file:
        js_file.write((
            f'var data = {{lines: {json.dumps(lines_info)}}};'
            f'var percent_low = 25;var percent_high = 75;'
            f'var header = {json.dumps(header)};'
            f'var merged_data = [];'
        ))

    with open(f'{str(path)}.kcov.html', 'w') as html_file:
        html_file.write(
            f'<script type="text/javascript" src="{path.name}.js"></script>'
        )
        html_file.write(kcov_html_template)


with open(f'{root}index.js', 'w') as js_file:
    js_file.write((
        f'var data = {{files: {json.dumps(index)}}};'
        f'var percent_low = 25;var percent_high = 75;'
        f'var header = {json.dumps(index_header)};'
        f'var merged_data = [];'
    ))

with open(f'{root}index.html', 'w') as html_file:
    html_file.write(kcov_index_html_template)