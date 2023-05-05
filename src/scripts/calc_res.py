
tt = 0
tf = 0
ft = 0
ff = 0

t1 = 11
t2 = 3
t3 = 20

# 计算精确度，准确率，召回率，漏报率，虚警率
def calc_res():
    print(tt, tf, ft, ff)
    print('精确度：', tt / (tt + tf))
    print('准确率：', (tt + ff) / (tt + ft + tf + ff))
    print('召回率：', tt / (tt + ft))
    print('漏报率：', ft / (tt + ft))
    print('虚警率：', tf / (tt + tf))


if __name__ == '__main__':
    tt = t1
    tf = t2
    ft = t3 - t1
    ff = 0
    calc_res()
