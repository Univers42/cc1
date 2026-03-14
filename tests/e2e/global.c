int counter = 10;

int increment(void) {
    counter = counter + 1;
    return counter;
}

int main(void) {
    increment();
    increment();
    increment();
    return counter;
}
