import os
import json
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

path = '/coverage/develop/.fuzzing.latest/custom-fuzzer/lcov'
p2p_sources = (
    'tezedge/networking/src',	
    'tezedge/networking/src/p2p',	
    'tezedge/networking/src/p2p/peer',
    'tezedge/tezos/encoding-derive/src',	
    'tezedge/tezos/encoding/src',	
    'tezedge/tezos/identity/src',	
    'tezedge/tezos/messages/src',	
    'tezedge/tezos/messages/src/base',	
    'tezedge/tezos/messages/src/p2p',	
    'tezedge/tezos/messages/src/p2p/encoding'
)	
rpc_sources = (
    'tezedge/rpc/src',
    'tezedge/rpc/src/encoding',
    'tezedge/rpc/src/server',	
    'tezedge/rpc/src/services'
)

html_template = """
<html>
<head>
  <title id="window-title">???</title>
  <link rel="stylesheet" href="/web-files/data/tablesorter-theme.css">
  <link rel="stylesheet" type="text/css" href="/web-files/data/bcov.css"/>
</head>

<noscript>
<font color=red><B>ERROR:</B></font> JavaScript need to be enabled for the coverage report to work.
</noscript>

<script type="text/javascript" src="index.js"></script>
<script type="text/javascript" src="/web-files/data/js/jquery.min.js"></script>
<script type="text/javascript" src="/web-files/data/js/tablesorter.min.js"></script>
<script type="text/javascript" src="/web-files/data/js/jquery.tablesorter.widgets.min.js"></script>
<script type="text/javascript" src="/web-files/data/js/handlebars.js"></script>
<script type="text/javascript" src="/web-files/data/js/kcov.js"></script>
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
        parser = MyHTMLParser(header, files)

        with open(f'{path}/{source}/index.html','r') as input_file:
            parser.feed(input_file.read())

        for file in files:
            title = file['title']
            file['link'] = f'../{source}/{title}.kcov.html'
            file['summary_name'] = f'[...]/code/{source}/{title}'

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

rpc_summary = generate_index('RPC-Fuzzer', rpc_sources)
p2p_summary = generate_index('P2P-Fuzzer', p2p_sources)

files = [
    generate_header('RPC-Fuzzer', rpc_summary),
    generate_header('P2P-Fuzzer', p2p_summary),
]
header = {
    'command' : 'Custom-Fuzzer',
    'date' : rpc_summary['date'],
    'instrumented' : rpc_summary['instrumented'] + p2p_summary['instrumented'],
    'covered' : rpc_summary['covered'] + p2p_summary['covered']
}

with open('/coverage/develop/.fuzzing.latest/custom-fuzzer/index.js', 'w') as js_file:
    js_file.write((
        f'var data = {{files: {json.dumps(files)}}};'
        f'var percent_low = 25;var percent_high = 75;'
        f'var header = {json.dumps(header)};'
        f'var merged_data = [];'
    ))

with open('/coverage/develop/.fuzzing.latest/custom-fuzzer/index.html', 'w') as html_file:
    html_file.write(html_template)


