import os
import json
import pathlib
from html.parser import HTMLParser
from enum import Enum


class Context(Enum):
    headerItem = 0
    headerLines = 1
    headerCovTableEntryHit = 2
    headerCovTableEntryTotal = 3
    headerDate = 4
    headerDateValue = 5
    headerCovTableEntryHitDone = 6
    headerCovTableEntryTotalDone = 7
    coverFile = 9
    coverPer = 10
    coverNum = 11
    Unknown = 12
    Skip = 13


class MyHTMLParser(HTMLParser):
    def __init__(self, header, files):
        super().__init__()
        self.current = Context.Unknown
        self.header = header
        self.files = files
        self.file = dict()

    def handle_starttag(self, tag, attrs):
        if tag == 'td':
            attrs = dict(attrs)

            try:
                if attrs['class'] == 'headerItem':
                    self.current = Context.headerItem

                if self.current == Context.headerCovTableEntryHitDone and \
                        attrs['class'] == 'headerCovTableEntry':
                    self.current = Context.headerCovTableEntryTotal

                if self.current == Context.headerLines and \
                        attrs['class'] == 'headerCovTableEntry':
                    self.current = Context.headerCovTableEntryHit

                if self.current == Context.headerDate and \
                        attrs['class'] == 'headerValue':
                    self.current = Context.headerDateValue

                if attrs['class'] == 'coverFile':
                    self.current = Context.coverFile

                if self.current != Context.Skip:
                    if attrs['class'] in ('coverPerLo', 'coverPerMed', 'coverPerHi'):
                        self.current = Context.coverPer

                    if attrs['class'] in ('coverNumLo', 'coverNumMed', 'coverNumHi'):
                        self.current = Context.coverNum
            except:
                pass

    def handle_data(self, data):
        if self.current == Context.headerItem:
            if data == 'Lines:':
                self.current = Context.headerLines

            if data == 'Date:':
                self.current = Context.headerDate

        if self.current == Context.headerCovTableEntryHit:
            self.header['covered'] += int(data)
            self.current = Context.headerCovTableEntryHitDone

        if self.current == Context.headerCovTableEntryTotal:
            self.header['instrumented'] += int(data)
            self.current = Context.headerCovTableEntryTotalDone

        if self.current == Context.headerDateValue:
            self.header['date'] = data
            self.current = Context.Unknown

        if self.current == Context.coverFile:
            if self.file:
                self.files.append(self.file)

            self.file = dict()
            self.file['title'] = data
            self.current = Context.Unknown

        if self.current == Context.coverPer:
            self.file['covered'] = data.rstrip('\xa0%')
            covered = float(self.file['covered'])

            if covered < 25:
                self.file['covered_class'] = 'lineNoCov'
            elif covered < 75:
                self.file['covered_class'] = 'linePartCov'
            else:
                self.file['covered_class'] = 'lineCov'

            self.current = Context.Unknown

        if self.current == Context.coverNum:
            covered, total = (int(x) for x in data.split('/'))
            self.file['covered_lines'] = str(covered)
            self.file['uncovered_lines'] = str(total - covered)
            self.file['total_lines'] = str(total)
            self.current = Context.Skip


path = '/coverage/develop/.fuzzing.latest/p2p-rpc-fuzzers/lcov'
p2p_sources = (
    'tezedge/networking/src',
    'tezedge/shell_automaton/src/peer',
    'tezedge/shell_automaton/src/peer/binary_message/read',
    'tezedge/shell_automaton/src/peer/binary_message/write',
    'tezedge/shell_automaton/src/peer/chunk/read',
    'tezedge/shell_automaton/src/peer/chunk/write',
    'tezedge/shell_automaton/src/peer/connection',
    'tezedge/shell_automaton/src/peer/connection/closed',
    'tezedge/shell_automaton/src/peer/connection/incoming',
    'tezedge/shell_automaton/src/peer/connection/incoming/accept',
    'tezedge/shell_automaton/src/peer/connection/outgoing',
    'tezedge/shell_automaton/src/peer/disconnection',
    'tezedge/shell_automaton/src/peer/handshaking',
    'tezedge/shell_automaton/src/peer/message/read',
    'tezedge/shell_automaton/src/peer/message/write',
    'tezedge/shell_automaton/src/peers',
    'tezedge/shell_automaton/src/peers/add',
    'tezedge/shell_automaton/src/peers/add/multi',
    'tezedge/shell_automaton/src/peers/check/timeouts',
    'tezedge/shell_automaton/src/peers/dns_lookup',
    'tezedge/shell_automaton/src/peers/graylist',
    'tezedge/shell_automaton/src/peers/remove',
    'tezedge/tezos/messages/src',
    'tezedge/tezos/messages/src/base',
    'tezedge/tezos/messages/src/p2p',
    'tezedge/tezos/messages/src/p2p/encoding'
)
rpc_sources = (
    'tezedge/rpc/src',
    'tezedge/rpc/src/encoding',
    'tezedge/rpc/src/server',
    'tezedge/rpc/src/services',
    'tezedge/rpc/src/services/protocol',
    'tezedge/rpc/src/services/protocol/proto_001',
    'tezedge/rpc/src/services/protocol/proto_002',
    'tezedge/rpc/src/services/protocol/proto_003',
    'tezedge/rpc/src/services/protocol/proto_004',
    'tezedge/rpc/src/services/protocol/proto_005_2',
    'tezedge/rpc/src/services/protocol/proto_006',
    'tezedge/rpc/src/services/protocol/proto_007',
    'tezedge/rpc/src/services/protocol/proto_008',
    'tezedge/rpc/src/services/protocol/proto_008_2',
    'tezedge/rpc/src/services/protocol/proto_009',
    'tezedge/rpc/src/services/protocol/proto_010',
    'tezedge/shell_automaton/src/rpc'
)

html_template = """
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
  <tr><td class="ruler"><img src="glass.png" width="3" height="3" alt=""/></td></tr>
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
  <tr><td class="ruler"><img src="glass.png" width="3" height="3" alt=""/></td></tr>
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
  <tr><td class="ruler"><img src="amber.png" width="3" height="3" alt=""/></td></tr>
  <tr><td class="versionInfo">Generated by: <a href="http://simonkagstrom.github.com/kcov/index.html">Kcov</a></td></tr>
</table>
</body>
</html>
"""


def generate_index(command, sources):
    header = {
        'command': command,
        'covered': 0,
        'instrumented': 0
    }
    files = []

    try:
        os.mkdir(f'{path}/{command}')
    except:
        pass

    for source in sources:
        _files = []
        parser = MyHTMLParser(header, _files)

        with open(f'{path}/{source}/index.html', 'r') as input_file:
            parser.feed(input_file.read())

        for file in _files:
            title = file['title']
            file['link'] = f'../{source}/{title}.kcov.html'
            file['summary_name'] = f'[...]/code/{source}/{title}'

        files += _files

    with open(f'{path}/{command}/index.js', 'w') as js_file:
        js_file.write((
            f'var data = {{files: {json.dumps(files)}}};'
            f'var percent_low = 25;var percent_high = 75;'
            f'var header = {json.dumps(header)};'
            f'var merged_data = [];'
        ))

    with open(f'{path}/{command}/index.html', 'w') as html_file:
        html_file.write(html_template)

    return header


def generate_header(command, summary):
    covered_lines = summary['covered']
    instrumented_lines = summary['instrumented']
    covered = (covered_lines / instrumented_lines) * 100
    uncovered_lines = instrumented_lines - covered_lines
    header = {
        'link': f'lcov/{command}/index.html',
        'title': command,
        'summary_name': command,
        'covered': f'{covered:.1f}',
        'covered_lines': str(covered_lines),
        'uncovered_lines': str(uncovered_lines),
        'total_lines': str(instrumented_lines)
    }

    if covered < 25:
        header['covered_class'] = 'lineNoCov'
    elif covered < 75:
        header['covered_class'] = 'linePartCov'
    else:
        header['covered_class'] = 'lineCov'

    return header


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


def coverage_files_summary(coverage):
    for file, counts in coverage:
        covered = sum(1 for (_, x) in counts if x != 0)
        total = len(counts)
        yield (file, total, covered)


def generate_ocaml_header():
    total_all = 0
    covered_all = 0

    for file, total, covered in coverage_files_summary(read_coverage()):
        total_all += total
        covered_all += covered

    return {
        'link': 'ocaml/index.html',
        'title': 'OCaml coverage',
        'summary_name': 'OCaml coverage',
        'covered': round((covered_all / total_all) * 100, 2),
        'covered_lines': str(covered_all),
        'uncovered_lines': str(total_all - covered_all),
        'total_lines': str(total_all),
    }


rpc_summary = generate_index('RPC-Fuzzer', rpc_sources)
p2p_summary = generate_index('P2P-Fuzzer', p2p_sources)

files = [
    generate_header('RPC-Fuzzer', rpc_summary),
    generate_header('P2P-Fuzzer', p2p_summary),
    generate_ocaml_header()
]
header = {
    'command': 'P2P-RPC-Fuzzers',
    'date': rpc_summary['date'],
    'instrumented': sum(int(x['total_lines']) for x in files),
    'covered': sum(int(x['covered_lines']) for x in files)
}

with open('/coverage/develop/.fuzzing.latest/p2p-rpc-fuzzers/index.js', 'w') as js_file:
    js_file.write((
        f'var data = {{files: {json.dumps(files)}}};'
        f'var percent_low = 25;var percent_high = 75;'
        f'var header = {json.dumps(header)};'
        f'var merged_data = [];'
    ))

with open('/coverage/develop/.fuzzing.latest/p2p-rpc-fuzzers/index.html', 'w') as html_file:
    html_file.write(html_template)
