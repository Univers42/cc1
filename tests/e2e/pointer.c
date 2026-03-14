int main(void) {
    int x = 42;
    int *p = &x;
    *p = 99;
    return x;
}
