import os
import json
import requests
from bs4 import BeautifulSoup
from tqdm import tqdm
from time import sleep


# 爬取网页内容：https://rustsec.org/keywords/，并保存到本地
def get_html():
    url = 'https://rustsec.org/keywords/'
    response = requests.get(url)
    content = response.text

    with open('data/keywords.html', 'w') as f:
        f.write(content)



def parse_html():
    with open('data/keywords.html', 'r') as f:
        content = f.read()
    soup = BeautifulSoup(content, 'html.parser')
    # 遍历ul的li标签 
    for i in tqdm(soup.find_all('li')):
        # 提取其中的href
        url = 'https://rustsec.org' + i.a['href']
        # 使用requests.get()获取网页内容
        content = requests.get(url).text
        # 获取该网页的名称
        name = i.a.text.split('/')[-1]
        # 保存到data/keywords/目录下
        with open('data/keywords/' + name + '.html', 'w') as f:
            f.write(content)
            print('save ' + name + ' to local successfully!')
        sleep(1)

def get_keywords():
    # 遍历data/keywords/目录下的所有文件
    names = []
    for i in tqdm(os.listdir('data/keywords/')):
        # 提取文件名
        name = i.split('.')[0]
        names.append(name)
    with open('data/keywords.txt', 'w') as f:
        f.write(', '.join(names))           


def check_keywords():
    # 读取data/keywords.txt文件
    with open('data/keywords.txt', 'r') as f:
        content = f.read()
        # 以逗号分隔
        keywords = content.split(',')
        # 去除空格
        keywords = [i.strip() for i in keywords]
        # 读取data/memory_keywords.txt文件
        with open('data/memory_keywords.txt', 'r') as f2:
            memory_keywords = f2.read()
            memory_keywords = memory_keywords.split('\n')
            memory_keywords = [i.strip() for i in memory_keywords]
            # 检查是否有遗漏
            for i in memory_keywords:
                if i not in keywords:
                    print(i)
                    print('there is a keyword missing!')
                    break
            else:
                print('all keywords are in keywords.txt')


def get_memory_bug_infos():
    # 读取data/memory_keywords.txt文件, 将关键字保存到keywords列表中
    keywords = []
    with open('data/memory_keywords.txt', 'r') as f:
        content = f.read()
        keywords = content.split('\n')
        keywords = [i.strip() for i in keywords]
    # 遍历data/keywords/目录下的所有文件
    for i in tqdm(os.listdir('data/keywords/')):
        # 提取文件名
        name = i.split('.')[0]
        # 如果文件名在keywords列表中
        if name in keywords:
            # 读取文件内容
            with open('data/keywords/' + i, 'r') as f:
                content = f.read()
                # 使用BeautifulSoup解析网页内容
                soup = BeautifulSoup(content, 'html.parser')
                # 获取所有的li标签
                lis = soup.find_all('li')
                records = []
                # 遍历li标签
                for li in lis:
                    # 提取time, a.href, p.text
                    time = li.time.text.strip()
                    href = 'https://rustsec.org' + li.a['href']
                    href = href.strip()
                    description = li.p.text.strip()
                    records.append({'time': time, 'href': href, 'description': description})
                # 将记录保存到data/memory_safety_statistics/ + 文件名 + .json
                with open('data/memory_safety_statistics/' + name + '.json', 'w') as f2:
                    json.dump(records, f2, indent=4)


# 根据data/memory_safety_statistics/目录下的所有文件，绘制漏洞数量的柱状图
# 其中，x轴为漏洞类型，即文件名，y轴为漏洞数量，即文件中的记录数量，每个文件中的记录为一个漏洞
# 使用不同颜色的图例表示不同的年份，例如，2019年的漏洞数量为红色，2020年的漏洞数量为蓝色，只考虑2021年到2023年的漏洞数量
# 将所有数据绘制在一张图上，保存到data/memory_safety_statistics.png
def draw_memory_safety_statistics_with_year():
    import matplotlib.pyplot as plt
    # 读取data/memory_safety_statistics/目录下的所有文件

    all_records = []
    for i in tqdm(os.listdir('data/memory_safety_statistics/')):
        # 读取文件内容
        with open('data/memory_safety_statistics/' + i, 'r') as f:
            records = json.load(f)
            # 根据年份统计每个类型的漏洞数量
            years = {'2021': 0, '2022': 0, '2023': 0}
            for record in records:
                year = record['time'].split(',')[-1].strip()
                if year in ['2021', '2022', '2023']:
                    years[year] += 1
            all_records.append({'kind': i.split('.')[0], 'counts': years})
    # 绘制柱状图，x轴为漏洞类型，其标签文字垂直显示，保证所有文字都能显示出来
    # y轴为漏洞数量，使用不同颜色的图例表示不同的年份
    # 将所有数据绘制在一张图上，保存到data/memory_safety_statistics.png
    kinds = [i['kind'] for i in all_records]
    counts_2021 = [i['counts']['2021'] for i in all_records]
    counts_2022 = [i['counts']['2022'] for i in all_records]
    counts_2023 = [i['counts']['2023'] for i in all_records]
    width = 0.4
    x_2021 = range(len(kinds))
    x_2022 = [x + width for x in x_2021]
    x_2023 = [x + width for x in x_2022]
    plt.bar(x_2021, counts_2021, width=width, label='2021', color='red')
    plt.bar(x_2022, counts_2022, width=width, label='2022', color='blue')
    plt.bar(x_2023, counts_2023, width=width, label='2023', color='green')
    plt.xticks([x + width for x in range(len(kinds))], kinds, rotation=90)
    plt.legend()
    plt.savefig('data/memory_safety_statistics.png')
    plt.show()


def draw_memory_safety_statistics(top=5):
    import matplotlib.pyplot as plt
    # 读取data/memory_safety_statistics/目录下的所有文件
    all_records = []
    for i in tqdm(os.listdir('data/memory_safety_statistics/')):
        # 读取文件内容
        with open('data/memory_safety_statistics/' + i, 'r') as f:
            records = json.load(f)
            cnt = 0
            for record in records:
                if record['time'].split(',')[-1].strip() in ['2021', '2022', '2023']:
                    cnt += 1
            all_records.append({'kind': i.split('.')[0], 'counts': cnt})
    # 根据漏洞数量从大到小排序
    all_records = sorted(all_records, key=lambda x: x['counts'], reverse=True)
    # 绘制柱状图，x轴为漏洞类型，其标签文字垂直显示，保证所有文字都能显示出来
    # y轴为漏洞数量，在柱状图上方显示数字
    # 将所有数据绘制在一张图上，保存到data/memory_safety_statistics.png
    top = min(top, len(all_records))
    kinds = [i['kind'] for i in all_records][0:top]
    counts = [i['counts'] for i in all_records][0:top]
    plt.bar(kinds, counts)
    plt.xticks(kinds, rotation=45)
    for x, y in zip(kinds, counts):
        plt.text(x, y + 0.05, '%d' % y, ha='center', va='bottom')
    plt.subplots_adjust(left=0.026, bottom=0.157, right=0.97, top=0.957, wspace=0.2, hspace=0.2)
    # plt.show()
    plt.gcf().set_size_inches(26.54, 13.80)
    plt.savefig('data/memory_safety_statistics_top' + str(top) + '.png', dpi=96)
    plt.tight_layout()
    

def get_all_records_count():
    count = 0
    for i in os.listdir('data/memory_safety_statistics/'):
        with open('data/memory_safety_statistics/' + i, 'r') as f:
            records = json.load(f)
            count += len(records)
    print(count)

def demo():
# 导入matplotlib库
    import matplotlib.pyplot as plt

# 假设你有以下记录
    records = [
        {"record_kind": "A", "counts": {"2020": 10, "2021": 15}},
        {"record_kind": "B", "counts": {"2020": 20, "2021": 25}},
        {"record_kind": "C", "counts": {"2020": 30, "2021": 35}}
    ]

# 提取记录类型和数量
    kinds = [r["record_kind"] for r in records]
    counts_2020 = [r["counts"]["2020"] for r in records]
    counts_2021 = [r["counts"]["2021"] for r in records]

# 设置柱子的宽度和位置
    width = 0.4
    x_2020 = range(len(kinds))
    x_2021 = [x + width for x in x_2020]

# 绘制柱状图
    plt.bar(x_2020, counts_2020, width=width, label="2020")
    plt.bar(x_2021, counts_2021, width=width, label="2021")

# 设置x轴的刻度和标签
    plt.xticks([x + width / 2 for x in x_2020], kinds)

# 设置y轴的标签
    plt.ylabel("Counts")

# 显示图例和标题
    plt.legend()
    plt.title("Bar chart example")

# 显示图形
    plt.show()
    


if __name__ == '__main__':
    draw_memory_safety_statistics(top=10)