import sys, zlib, numpy as np
from sklearn.linear_model import LogisticRegression
# Fits the suggestion ranking weights. Usage: cargo run --release -p open365-spell --example pool_dump > pool.tsv; python -I fit_ranking.py pool.tsv
NF = 21; NS = 16
qs = {}
for line in open(sys.argv[1], encoding='utf-8'):
    c = line.rstrip('\n').split('\t')
    key = (c[0], c[1])
    rank = int(c[2]); f = [float(x) for x in c[4:4 + NF]]; st = int(float(c[4 + NF]))
    qs.setdefault(key, []).append((rank, f, st, c[3]))
def vec(f, st):
    oh = [0.0] * NS; oh[st] = 1.0
    return f + oh
keys = sorted(qs)
def is_test(k):  # hold out by word across locales
    return zlib.crc32(k[1].encode()) % 4 == 0
train = [k for k in keys if not is_test(k)]
test = [k for k in keys if is_test(k)]
def evaluate(ks, w):
    n = t1 = t3 = t5 = 0
    for k in ks:
        cands = qs[k]
        X = np.array([vec(f, st) for r, f, st, _ in cands])
        sc = X @ w if w is not None else -np.arange(len(cands), dtype=float)
        order = np.argsort(-sc, kind='stable')
        ranks = [cands[i][0] for i in order]
        n += 1
        t1 += ranks[0] == 0
        t3 += 0 in ranks[:3]
        t5 += 0 in ranks[:5]
    return 100 * t1 / n, 100 * t3 / n, 100 * t5 / n, n
print('baseline train', evaluate(train, None)); print('baseline test', evaluate(test, None))
def pairs(ks, topw):
    A = []; y = []; wt = []
    for k in ks:
        cands = qs[k]
        X = [np.array(vec(f, st)) for r, f, st, _ in cands]
        for i, (ri, *_ ) in enumerate(cands):
            for j, (rj, *_ ) in enumerate(cands):
                if ri < rj:
                    A.append(X[i] - X[j]); y.append(1); wt.append(1.0 / (1 + ri) ** topw)
                    A.append(X[j] - X[i]); y.append(0); wt.append(1.0 / (1 + ri) ** topw)
    return np.array(A), np.array(y), np.array(wt)
best = None
for topw in (1.0, 2.0):
    A, y, wt = pairs(train, topw)
    for C in (0.01, 0.1, 1.0):
        m = LogisticRegression(C=C, fit_intercept=False, max_iter=2000)
        m.fit(A, y, sample_weight=wt)
        w = m.coef_[0]
        tr = evaluate(train, w); te = evaluate(test, w)
        print(topw, C, 'train', tr, 'test', te)
        if best is None or te[0] + te[1] > best[0]:
            best = (te[0] + te[1], topw, C, w)
_, topw, C, w = best
print('best', topw, C)
A, y, wt = pairs(keys, topw)
m = LogisticRegression(C=C, fit_intercept=False, max_iter=2000).fit(A, y, sample_weight=wt)
w = m.coef_[0]
print('all', evaluate(keys, w))
print('WEIGHTS', [round(float(x), 3) for x in w[:NF]])
print('STAGE_BIAS', [round(float(x), 3) for x in w[NF:]])
